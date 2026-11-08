use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Mutex, OnceLock, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Notify;

const SAVE_EVERY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredGame {
    #[serde(default)]
    pub application_id: Option<u64>,
    pub name: String,
    pub exe: String,
    #[serde(default)]
    pub last_played_ms: u64,
    #[serde(default = "detection_default")]
    pub detection: bool,
}

fn detection_default() -> bool {
    true
}

impl RegisteredGame {
    pub fn verified(&self) -> bool {
        self.application_id.is_some()
    }

    fn matches(&self, key: &GameKey) -> bool {
        match key {
            GameKey::Application(id) => self.application_id == Some(*id),
            GameKey::Program(exe) => self.application_id.is_none() && same_exe(&self.exe, exe),
        }
    }

    pub fn key(&self) -> GameKey {
        match self.application_id {
            Some(id) => GameKey::Application(id),
            None => GameKey::Program(self.exe.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum GameKey {
    Application(u64),
    Program(String),
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    games: Vec<RegisteredGame>,
}

struct Registry {
    games: Vec<RegisteredGame>,
    saved_at: Option<Instant>,
    dirty: bool,
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
static REVISION: AtomicU64 = AtomicU64::new(0);
static CHANGED: Notify = Notify::const_new();

fn path() -> Option<PathBuf> {
    crate::settings::data_dir().map(|dir| dir.join("games.json"))
}

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| {
        let games = path()
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<Stored>(&bytes).ok())
            .map(|stored| stored.games)
            .unwrap_or_default();
        Mutex::new(Registry {
            games,
            saved_at: None,
            dirty: false,
        })
    })
}

fn with<T>(change: impl FnOnce(&mut Registry) -> T) -> T {
    let mut registry = registry().lock().unwrap_or_else(PoisonError::into_inner);
    change(&mut registry)
}

fn save(registry: &mut Registry) {
    registry.saved_at = Some(Instant::now());
    let Some(path) = path() else {
        registry.dirty = false;
        return;
    };
    let stored = Stored {
        games: registry.games.clone(),
    };
    let Ok(bytes) = serde_json::to_vec_pretty(&stored) else {
        return;
    };
    let temporary = path.with_extension("json.tmp");
    let written = path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(&temporary, bytes))
        .and_then(|()| fs::rename(&temporary, &path));
    match written {
        Ok(()) => registry.dirty = false,
        Err(error) => log::warn!("Could not save registered games: {error}"),
    }
}

fn edited(registry: &mut Registry) {
    registry.dirty = true;
    save(registry);
    REVISION.fetch_add(1, Ordering::Relaxed);
    CHANGED.notify_waiters();
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

pub(super) fn same_exe(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

pub(super) fn normalize_exe(path: &str) -> String {
    path.replace('\\', "/")
}

pub fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

pub(super) async fn changed() {
    CHANGED.notified().await;
}

pub fn games() -> Vec<RegisteredGame> {
    let mut games = with(|registry| registry.games.clone());
    games.sort_by_key(|game| std::cmp::Reverse(game.last_played_ms));
    games
}

pub fn set_detection(key: &GameKey, on: bool) {
    with(|registry| {
        if let Some(game) = registry.games.iter_mut().find(|game| game.matches(key)) {
            game.detection = on;
            edited(registry);
        }
    });
}

pub fn rename(key: &GameKey, name: &str) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    with(|registry| {
        if let Some(game) = registry
            .games
            .iter_mut()
            .find(|game| game.matches(key) && !game.verified())
        {
            game.name = name.to_owned();
            edited(registry);
        }
    });
}

pub fn remove(key: &GameKey) {
    with(|registry| {
        registry.games.retain(|game| !game.matches(key));
        edited(registry);
    });
}

pub fn add_program(name: &str, exe: &str) {
    let exe = normalize_exe(exe);
    with(|registry| {
        let key = GameKey::Program(exe.clone());
        match registry.games.iter_mut().find(|game| game.matches(&key)) {
            Some(game) => game.detection = true,
            None => registry.games.push(RegisteredGame {
                application_id: None,
                name: name.to_owned(),
                exe,
                last_played_ms: now_ms(),
                detection: true,
            }),
        }
        edited(registry);
    });
}

pub(super) fn programs() -> Vec<(String, String)> {
    with(|registry| {
        registry
            .games
            .iter()
            .filter(|game| !game.verified() && game.detection)
            .map(|game| (game.name.clone(), game.exe.clone()))
            .collect()
    })
}

pub(super) fn detection_off(application_id: u64) -> bool {
    with(|registry| {
        registry
            .games
            .iter()
            .any(|game| game.application_id == Some(application_id) && !game.detection)
    })
}

pub(super) fn played(seen: &[(Option<u64>, String, String)]) {
    if seen.is_empty() {
        return;
    }
    let now = now_ms();
    with(|registry| {
        let mut added = false;
        for (application_id, name, exe) in seen {
            let key = match application_id {
                Some(id) => GameKey::Application(*id),
                None => GameKey::Program(exe.clone()),
            };
            match registry.games.iter_mut().find(|game| game.matches(&key)) {
                Some(game) => {
                    game.last_played_ms = now;
                    if game.verified() {
                        game.exe.clone_from(exe);
                    }
                }
                None => {
                    added = true;
                    registry.games.push(RegisteredGame {
                        application_id: *application_id,
                        name: name.clone(),
                        exe: exe.clone(),
                        last_played_ms: now,
                        detection: true,
                    });
                }
            }
        }
        registry.dirty = true;
        let due = registry
            .saved_at
            .is_none_or(|saved| saved.elapsed() >= SAVE_EVERY);
        if added || due {
            save(registry);
        }
        REVISION.fetch_add(1, Ordering::Relaxed);
    });
}

pub fn flush() {
    with(|registry| {
        if registry.dirty {
            save(registry);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(application_id: Option<u64>, exe: &str) -> RegisteredGame {
        RegisteredGame {
            application_id,
            name: "Game".into(),
            exe: exe.into(),
            last_played_ms: 0,
            detection: true,
        }
    }

    #[test]
    fn verified_games_match_by_application_and_programs_by_path_in_any_case() {
        let verified = game(Some(7), "C:/Games/a.exe");
        assert!(verified.matches(&GameKey::Application(7)));
        assert!(!verified.matches(&GameKey::Program("C:/Games/a.exe".into())));
        let program = game(None, "C:/Games/b.exe");
        assert!(program.matches(&GameKey::Program("c:/games/B.EXE".into())));
        assert!(!program.matches(&GameKey::Application(7)));
    }

    #[test]
    fn stored_lists_default_detection_on() {
        let stored: Stored =
            serde_json::from_str(r#"{"games":[{"name":"X","exe":"C:/x.exe"}]}"#).unwrap();
        assert!(stored.games[0].detection);
        assert_eq!(stored.games[0].application_id, None);
    }
}
