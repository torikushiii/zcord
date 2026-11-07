use super::{
    catalog::Catalog,
    registry::{self, GameKey},
};
use std::{collections::HashMap, sync::Arc, time::Duration};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tokio::sync::watch;

const INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Detected {
    pub(crate) application_id: Option<u64>,
    pub(crate) name: String,
    pub(crate) start_ms: u64,
}

#[derive(Clone)]
struct Candidate {
    application_id: Option<u64>,
    name: String,
    exe: String,
    start_ms: u64,
}

pub(crate) async fn run(
    mut catalog: watch::Receiver<Option<Arc<Catalog>>>,
    output: watch::Sender<Option<Detected>>,
) {
    let mut seen = HashMap::new();
    loop {
        // Programs added by hand need no catalog, so the scan runs before one arrives too.
        let current = catalog.borrow().clone();
        let previous = seen.clone();
        let detected =
            match tokio::task::spawn_blocking(move || scan(current.as_deref(), &previous)).await {
                Ok((starts, detected)) => {
                    seen = starts;
                    detected
                }
                Err(error) => {
                    log::warn!("Could not scan for games: {error}");
                    seen.clear();
                    None
                }
            };
        output.send_if_modified(|shown| {
            if *shown == detected {
                false
            } else {
                *shown = detected.clone();
                true
            }
        });
        tokio::select! {
            () = tokio::time::sleep(INTERVAL) => {}
            () = registry::changed() => {}
            result = catalog.changed() => {
                if result.is_err() {
                    break;
                }
            }
        }
    }
}

fn scan(
    catalog: Option<&Catalog>,
    previous: &HashMap<GameKey, u64>,
) -> (HashMap<GameKey, u64>, Option<Detected>) {
    let programs = registry::programs();
    if catalog.is_none() && programs.is_empty() {
        return (HashMap::new(), None);
    }
    let mut system = System::new();
    let refresh = ProcessRefreshKind::new()
        .with_exe(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet)
        .with_user(UpdateKind::OnlyIfNotSet);
    system.refresh_processes_specifics(ProcessesToUpdate::All, refresh);
    let own_user = sysinfo::get_current_pid()
        .ok()
        .and_then(|pid| system.process(pid))
        .and_then(|process| process.user_id())
        .cloned();
    let Some(own_user) = own_user else {
        return (HashMap::new(), None);
    };
    let mut found: HashMap<GameKey, Candidate> = HashMap::new();
    for process in system.processes().values() {
        if process.user_id() != Some(&own_user) {
            continue;
        }
        let Some(executable) = process.exe() else {
            continue;
        };
        let path = executable.to_string_lossy();
        let exe = registry::normalize_exe(&path);
        let command_line = || {
            process
                .cmd()
                .iter()
                .map(|argument| argument.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let start_ms = process.start_time().saturating_mul(1000);
        let (key, candidate) = match programs
            .iter()
            .find(|(_, program)| registry::same_exe(program, &exe))
        {
            Some((name, program)) => (
                GameKey::Program(program.clone()),
                Candidate {
                    application_id: None,
                    name: name.clone(),
                    exe,
                    start_ms,
                },
            ),
            None => {
                let Some(game) = catalog.and_then(|catalog| catalog.matches(&path, command_line))
                else {
                    continue;
                };
                if registry::detection_off(game.id) {
                    continue;
                }
                (
                    GameKey::Application(game.id),
                    Candidate {
                        application_id: Some(game.id),
                        name: game.name.clone(),
                        exe,
                        start_ms,
                    },
                )
            }
        };
        found
            .entry(key)
            .and_modify(|current| {
                if candidate.start_ms > current.start_ms {
                    *current = candidate.clone();
                }
            })
            .or_insert(candidate);
    }
    let seen: Vec<_> = found
        .values()
        .map(|game| (game.application_id, game.name.clone(), game.exe.clone()))
        .collect();
    registry::played(&seen);
    let starts = stable_starts(&found, previous);
    let detected = select(&found, &starts);
    (starts, detected)
}

fn stable_starts(
    found: &HashMap<GameKey, Candidate>,
    previous: &HashMap<GameKey, u64>,
) -> HashMap<GameKey, u64> {
    found
        .iter()
        .map(|(key, candidate)| {
            (
                key.clone(),
                previous.get(key).copied().unwrap_or(candidate.start_ms),
            )
        })
        .collect()
}

fn select(found: &HashMap<GameKey, Candidate>, starts: &HashMap<GameKey, u64>) -> Option<Detected> {
    found
        .iter()
        .map(|(key, candidate)| Detected {
            application_id: candidate.application_id,
            name: candidate.name.clone(),
            start_ms: starts.get(key).copied().unwrap_or(candidate.start_ms),
        })
        .max_by(|left, right| {
            left.start_ms
                .cmp(&right.start_ms)
                .then_with(|| right.application_id.cmp(&left.application_id))
                .then_with(|| right.name.cmp(&left.name))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: u64, name: &str, start_ms: u64) -> (GameKey, Candidate) {
        (
            GameKey::Application(id),
            Candidate {
                application_id: Some(id),
                name: name.into(),
                exe: format!("C:/{name}.exe"),
                start_ms,
            },
        )
    }

    #[test]
    fn start_times_stay_stable_until_a_game_disappears() {
        let key = GameKey::Application(7);
        let found = HashMap::from([candidate(7, "Game", 1_000)]);
        let starts = stable_starts(&found, &HashMap::new());
        assert_eq!(starts[&key], 1_000);
        let restarted = HashMap::from([candidate(7, "Game", 9_000)]);
        let starts = stable_starts(&restarted, &starts);
        assert_eq!(starts[&key], 1_000);
        let vanished = stable_starts(&HashMap::new(), &starts);
        assert!(vanished.is_empty());
        let reappeared = stable_starts(&restarted, &vanished);
        assert_eq!(reappeared[&key], 9_000);
    }

    #[test]
    fn the_most_recently_started_game_wins_ties_by_id() {
        let found = HashMap::from([
            candidate(4, "Four", 5_000),
            candidate(2, "Two", 9_000),
            candidate(3, "Three", 9_000),
        ]);
        let starts = stable_starts(&found, &HashMap::new());
        let detected = select(&found, &starts).expect("a game is selected");
        assert_eq!(detected.application_id, Some(2));
        assert_eq!(detected.name, "Two");
        assert_eq!(detected.start_ms, 9_000);
    }

    #[test]
    fn stable_starts_are_used_for_selection() {
        let found = HashMap::from([candidate(1, "Older", 9_000), candidate(2, "Newer", 10_000)]);
        let previous = HashMap::from([(GameKey::Application(1), 1_000)]);
        let starts = stable_starts(&found, &previous);
        assert_eq!(select(&found, &starts).unwrap().application_id, Some(2));
        let previous = HashMap::from([(GameKey::Application(2), 1)]);
        let starts = stable_starts(&found, &previous);
        assert_eq!(select(&found, &starts).unwrap().application_id, Some(1));
    }
}
