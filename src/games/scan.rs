use super::catalog::Catalog;
use std::{collections::HashMap, sync::Arc, time::Duration};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tokio::sync::watch;

const INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Detected {
    pub(crate) application_id: u64,
    pub(crate) name: String,
    pub(crate) start_ms: u64,
}

#[derive(Clone)]
struct Candidate {
    id: u64,
    name: String,
    start_ms: u64,
}

pub(crate) async fn run(
    mut catalog: watch::Receiver<Option<Arc<Catalog>>>,
    output: watch::Sender<Option<Detected>>,
) {
    let mut seen = HashMap::new();
    loop {
        let current = catalog.borrow().clone();
        let detected = match current {
            Some(catalog) => {
                let previous = seen.clone();
                match tokio::task::spawn_blocking(move || scan(&catalog, &previous)).await {
                    Ok((starts, detected)) => {
                        seen = starts;
                        detected
                    }
                    Err(error) => {
                        log::warn!("Could not scan for games: {error}");
                        seen.clear();
                        None
                    }
                }
            }
            None => {
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
            result = catalog.changed() => {
                if result.is_err() {
                    break;
                }
            }
        }
    }
}

fn scan(catalog: &Catalog, previous: &HashMap<u64, u64>) -> (HashMap<u64, u64>, Option<Detected>) {
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
    let mut found: HashMap<u64, Candidate> = HashMap::new();
    for process in system.processes().values() {
        if process.user_id() != Some(&own_user) {
            continue;
        }
        let Some(executable) = process.exe() else {
            continue;
        };
        let path = executable.to_string_lossy();
        let command_line = || {
            process
                .cmd()
                .iter()
                .map(|argument| argument.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let Some(game) = catalog.matches(&path, command_line) else {
            continue;
        };
        let candidate = Candidate {
            id: game.id,
            name: game.name.clone(),
            start_ms: process.start_time().saturating_mul(1000),
        };
        found
            .entry(candidate.id)
            .and_modify(|current| {
                if candidate.start_ms > current.start_ms {
                    *current = candidate.clone();
                }
            })
            .or_insert(candidate);
    }
    let starts = stable_starts(&found, previous);
    let detected = select(&found, &starts);
    (starts, detected)
}

fn stable_starts(
    found: &HashMap<u64, Candidate>,
    previous: &HashMap<u64, u64>,
) -> HashMap<u64, u64> {
    found
        .iter()
        .map(|(id, candidate)| (*id, previous.get(id).copied().unwrap_or(candidate.start_ms)))
        .collect()
}

fn select(found: &HashMap<u64, Candidate>, starts: &HashMap<u64, u64>) -> Option<Detected> {
    found
        .iter()
        .map(|(id, candidate)| Detected {
            application_id: *id,
            name: candidate.name.clone(),
            start_ms: starts.get(id).copied().unwrap_or(candidate.start_ms),
        })
        .max_by(|left, right| {
            left.start_ms
                .cmp(&right.start_ms)
                .then_with(|| right.application_id.cmp(&left.application_id))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: u64, name: &str, start_ms: u64) -> Candidate {
        Candidate {
            id,
            name: name.into(),
            start_ms,
        }
    }

    #[test]
    fn start_times_stay_stable_until_a_game_disappears() {
        let found = HashMap::from([(7, candidate(7, "Game", 1_000))]);
        let starts = stable_starts(&found, &HashMap::new());
        assert_eq!(starts[&7], 1_000);
        let restarted = HashMap::from([(7, candidate(7, "Game", 9_000))]);
        let starts = stable_starts(&restarted, &starts);
        assert_eq!(starts[&7], 1_000);
        let vanished = stable_starts(&HashMap::new(), &starts);
        assert!(vanished.is_empty());
        let reappeared = stable_starts(&restarted, &vanished);
        assert_eq!(reappeared[&7], 9_000);
    }

    #[test]
    fn the_most_recently_started_game_wins_ties_by_id() {
        let found = HashMap::from([
            (4, candidate(4, "Four", 5_000)),
            (2, candidate(2, "Two", 9_000)),
            (3, candidate(3, "Three", 9_000)),
        ]);
        let starts = stable_starts(&found, &HashMap::new());
        let detected = select(&found, &starts).expect("a game is selected");
        assert_eq!(detected.application_id, 2);
        assert_eq!(detected.name, "Two");
        assert_eq!(detected.start_ms, 9_000);
    }

    #[test]
    fn stable_starts_are_used_for_selection() {
        let found = HashMap::from([
            (1, candidate(1, "Older", 9_000)),
            (2, candidate(2, "Newer", 10_000)),
        ]);
        let previous = HashMap::from([(1, 1_000)]);
        let starts = stable_starts(&found, &previous);
        assert_eq!(select(&found, &starts).unwrap().application_id, 2);
        let previous = HashMap::from([(2, 1)]);
        let starts = stable_starts(&found, &previous);
        assert_eq!(select(&found, &starts).unwrap().application_id, 1);
    }
}
