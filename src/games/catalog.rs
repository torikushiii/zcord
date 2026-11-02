use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::sync::watch;

const URL: &str = "https://discord.com/api/v9/applications/detectable";
const MAX_BODY: usize = 32 * 1024 * 1024;
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY: Duration = Duration::from_secs(60 * 60);
const CACHE_FILE: &str = "detectable-games.json";

pub(crate) struct GameInfo {
    pub(crate) id: u64,
    pub(crate) name: String,
}

pub(crate) struct Rule {
    name: String,
    exact: bool,
    arguments: Option<String>,
    game: Arc<GameInfo>,
}

pub(crate) struct Catalog {
    rules: HashMap<String, Vec<Rule>>,
    by_id: HashMap<u64, Arc<GameInfo>>,
}

#[derive(Deserialize, Serialize)]
struct CachedGame {
    id: u64,
    name: String,
    executables: Vec<CachedExecutable>,
}

#[derive(Deserialize, Serialize)]
struct CachedExecutable {
    name: String,
    exact: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    arguments: Option<String>,
}

impl Catalog {
    pub(crate) fn name(&self, id: u64) -> Option<&str> {
        self.by_id.get(&id).map(|game| game.name.as_str())
    }

    pub(crate) fn matches(
        &self,
        path: &str,
        command_line: impl FnOnce() -> String,
    ) -> Option<Arc<GameInfo>> {
        let path = normalize(path);
        let file = path.rsplit('/').next()?;
        let rules = self.rules.get(file)?;
        let command = rules
            .iter()
            .any(|rule| rule.arguments.is_some())
            .then(|| command_line().to_lowercase());
        rules.iter().find_map(|rule| {
            let name_matches = if rule.exact {
                rule.name == file
            } else {
                path == rule.name
                    || path
                        .strip_suffix(&rule.name)
                        .is_some_and(|prefix| prefix.ends_with('/'))
            };
            let arguments_match = rule.arguments.as_ref().is_none_or(|expected| {
                command
                    .as_deref()
                    .is_some_and(|command| command.contains(expected))
            });
            (name_matches && arguments_match).then(|| rule.game.clone())
        })
    }

    fn from_cached(games: Vec<CachedGame>) -> Self {
        let mut catalog = Self {
            rules: HashMap::new(),
            by_id: HashMap::new(),
        };
        for game in games {
            let mut executables = Vec::new();
            for executable in game.executables {
                let name = normalize(&executable.name);
                let Some(file) = name.rsplit('/').next().filter(|file| !file.is_empty()) else {
                    continue;
                };
                executables.push((
                    file.to_owned(),
                    name,
                    executable.exact,
                    executable
                        .arguments
                        .map(|arguments| arguments.to_lowercase()),
                ));
            }
            if executables.is_empty() {
                continue;
            }
            let info = catalog
                .by_id
                .entry(game.id)
                .or_insert_with(|| {
                    Arc::new(GameInfo {
                        id: game.id,
                        name: game.name.clone(),
                    })
                })
                .clone();
            for (file, name, exact, arguments) in executables {
                catalog.rules.entry(file).or_default().push(Rule {
                    name,
                    exact,
                    arguments,
                    game: info.clone(),
                });
            }
        }
        catalog
    }

    fn cached(&self) -> Vec<CachedGame> {
        let mut games: HashMap<u64, CachedGame> = HashMap::new();
        for rule in self.rules.values().flatten() {
            let game = games.entry(rule.game.id).or_insert_with(|| CachedGame {
                id: rule.game.id,
                name: rule.game.name.clone(),
                executables: Vec::new(),
            });
            game.executables.push(CachedExecutable {
                name: rule.name.clone(),
                exact: rule.exact,
                arguments: rule.arguments.clone(),
            });
        }
        let mut games: Vec<CachedGame> = games.into_values().collect();
        games.sort_by_key(|game| game.id);
        games
    }
}

pub(crate) fn decode(bytes: &[u8]) -> Option<Catalog> {
    let entries: Vec<Value> = serde_json::from_slice(bytes).ok()?;
    let games: Vec<CachedGame> = entries.iter().filter_map(parse_game).collect();
    (!games.is_empty()).then(|| Catalog::from_cached(games))
}

fn decode_cached(bytes: &[u8]) -> Option<Catalog> {
    let games: Vec<CachedGame> = serde_json::from_slice(bytes).ok()?;
    (!games.is_empty()).then(|| Catalog::from_cached(games))
}

fn parse_game(entry: &Value) -> Option<CachedGame> {
    let id = match entry.get("id")? {
        Value::String(id) => id.parse().ok()?,
        Value::Number(id) => id.as_u64()?,
        _ => return None,
    };
    let name = entry.get("name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let executables: Vec<CachedExecutable> = entry
        .get("executables")?
        .as_array()?
        .iter()
        .filter_map(parse_executable)
        .collect();
    (!executables.is_empty()).then(|| CachedGame {
        id,
        name: name.to_owned(),
        executables,
    })
}

fn parse_executable(entry: &Value) -> Option<CachedExecutable> {
    if entry
        .get("is_launcher")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let os = entry.get("os")?.as_str()?;
    if !os_allowed(os) {
        return None;
    }
    let name = entry.get("name")?.as_str()?;
    let (name, exact) = split_rule(name)?;
    let arguments = match entry.get("arguments") {
        None | Some(Value::Null) => None,
        Some(Value::String(arguments)) => (!arguments.is_empty()).then(|| arguments.to_lowercase()),
        Some(_) => return None,
    };
    Some(CachedExecutable {
        name,
        exact,
        arguments,
    })
}

fn os_allowed(os: &str) -> bool {
    if cfg!(target_os = "linux") {
        matches!(os, "linux" | "win32")
    } else if cfg!(target_os = "macos") {
        os == "darwin"
    } else if cfg!(target_os = "windows") {
        os == "win32"
    } else {
        false
    }
}

fn normalize(value: &str) -> String {
    value.replace('\\', "/").to_lowercase()
}

fn split_rule(name: &str) -> Option<(String, bool)> {
    let exact = name.starts_with('>');
    let name = name.strip_prefix('>').unwrap_or(name);
    let name = normalize(name);
    let file = name.rsplit('/').next()?;
    (!file.is_empty()).then_some((name, exact))
}

fn cache_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|directory| directory.join("zcord").join(CACHE_FILE))
}

fn read_cache(path: &Path) -> Option<(Catalog, Duration)> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > MAX_BODY as u64 {
        return None;
    }
    let age = SystemTime::now()
        .duration_since(metadata.modified().ok()?)
        .ok()?;
    let bytes = fs::read(path).ok()?;
    Some((decode_cached(&bytes)?, age))
}

fn write_cache(path: &Path, catalog: &Catalog) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    let bytes = serde_json::to_vec(&catalog.cached()).map_err(std::io::Error::other)?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)
}

async fn download(client: &reqwest::Client) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(URL)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY as u64)
    {
        return Err("list exceeds 32 MiB".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if body.len().saturating_add(chunk.len()) > MAX_BODY {
            return Err("list exceeds 32 MiB".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(crate) async fn run(output: watch::Sender<Option<Arc<Catalog>>>) {
    let client = reqwest::Client::new();
    let path = cache_path();
    loop {
        if let Some((catalog, age)) = path.as_deref().and_then(read_cache) {
            if output.borrow().is_none() {
                output.send_replace(Some(Arc::new(catalog)));
            }
            if age < MAX_AGE {
                tokio::time::sleep(MAX_AGE - age).await;
            }
        }
        let body = match download(&client).await {
            Ok(body) => body,
            Err(error) => {
                log::warn!("Could not load the detectable game list: {error}");
                tokio::time::sleep(RETRY).await;
                continue;
            }
        };
        let catalog = match tokio::task::spawn_blocking(move || decode(&body)).await {
            Ok(Some(catalog)) => catalog,
            Ok(None) => {
                log::warn!("The detectable game list was empty or invalid");
                tokio::time::sleep(RETRY).await;
                continue;
            }
            Err(error) => {
                log::warn!("Could not parse the detectable game list: {error}");
                tokio::time::sleep(RETRY).await;
                continue;
            }
        };
        if let Some(path) = &path {
            if let Err(error) = write_cache(path, &catalog) {
                log::warn!("Could not cache the detectable game list: {error}");
            }
        }
        output.send_replace(Some(Arc::new(catalog)));
        tokio::time::sleep(MAX_AGE).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn host_os() -> &'static str {
        if cfg!(target_os = "windows") {
            "win32"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        }
    }

    fn catalog(entries: Value) -> Catalog {
        decode(entries.to_string().as_bytes()).expect("synthetic catalog")
    }

    #[test]
    fn keeps_this_platforms_executables_and_skips_bad_entries() {
        let wrong_os = if host_os() == "darwin" {
            "linux"
        } else {
            "darwin"
        };
        let catalog = catalog(json!([
            {"id": "1", "name": "Kept", "executables": [
                {"name": "Kept.EXE", "os": host_os()},
                {"name": "launcher.exe", "os": host_os(), "is_launcher": true},
                {"name": "other.exe", "os": "unknown"},
                {"name": "wrong.exe", "os": wrong_os},
                {"name": "no-os.exe"}
            ]},
            {"id": "2", "name": "No executables"},
            {"id": "bad", "name": "Bad id", "executables": [{"name": "bad.exe", "os": host_os()}]},
            {"name": "No id", "executables": [{"name": "none.exe", "os": host_os()}]},
            {"id": "5", "name": "Bad arguments", "executables": [
                {"name": "badargs.exe", "os": host_os(), "arguments": 42}
            ]}
        ]));
        assert!(catalog.matches("/tmp/kept.exe", String::new).is_some());
        assert!(catalog.matches("/tmp/launcher.exe", String::new).is_none());
        assert!(catalog.matches("/tmp/other.exe", String::new).is_none());
        assert!(catalog.matches("/tmp/bad.exe", String::new).is_none());
        assert!(catalog.matches("/tmp/none.exe", String::new).is_none());
        assert!(catalog.matches("/tmp/badargs.exe", String::new).is_none());
        assert_eq!(catalog.name(1).as_deref(), Some("Kept"));
        assert_eq!(catalog.name(2), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn wine_executables_are_kept_on_linux() {
        let catalog = catalog(json!([
            {"id": "1", "name": "Windows game", "executables": [
                {"name": "game.exe", "os": "win32"}
            ]}
        ]));
        assert!(catalog.matches("/steam/game.exe", String::new).is_some());
    }

    #[test]
    fn names_are_normalized_and_exact_matches_keep_their_flag() {
        assert_eq!(
            split_rule("Game\\Bin\\Game.EXE"),
            Some(("game/bin/game.exe".into(), false))
        );
        assert_eq!(split_rule(">Game.EXE"), Some(("game.exe".into(), true)));
        assert_eq!(split_rule(">>Game.EXE"), Some((">game.exe".into(), true)));
        assert_eq!(split_rule(">"), None);
        assert_eq!(split_rule("dir/"), None);
    }

    #[test]
    fn matches_executables_by_path_suffix() {
        let catalog = catalog(json!([
            {"id": "1", "name": "Game", "executables": [
                {"name": "Game/Bin/Game.exe", "os": host_os()},
                {"name": "bare.exe", "os": host_os()}
            ]}
        ]));
        assert!(
            catalog
                .matches("/opt/Game/Bin/Game.exe", String::new)
                .is_some()
        );
        assert!(catalog.matches("Game/Bin/Game.exe", String::new).is_some());
        assert!(
            catalog
                .matches("/opt/NotGame/Bin/Game.exe", String::new)
                .is_none()
        );
        assert!(
            catalog
                .matches("C:\\Opt\\Game\\Bin\\Game.exe", String::new)
                .is_some()
        );
        assert!(catalog.matches("/opt/bare.exe", String::new).is_some());
        assert!(catalog.matches("/opt/notbare.exe", String::new).is_none());
    }

    #[test]
    fn exact_names_compare_against_the_file_name_only() {
        let exact = catalog(json!([
            {"id": "1", "name": "Game", "executables": [
                {"name": ">dir/game.exe", "os": host_os()}
            ]}
        ]));
        assert!(exact.matches("/opt/dir/game.exe", String::new).is_none());
        let suffix = catalog(json!([
            {"id": "1", "name": "Game", "executables": [
                {"name": "dir/game.exe", "os": host_os()}
            ]}
        ]));
        assert!(suffix.matches("/opt/dir/game.exe", String::new).is_some());
        let bare = catalog(json!([
            {"id": "1", "name": "Game", "executables": [
                {"name": ">game.exe", "os": host_os()}
            ]}
        ]));
        assert!(bare.matches("/opt/game.exe", String::new).is_some());
    }

    #[test]
    fn arguments_are_required_and_case_insensitive() {
        let catalog = catalog(json!([
            {"id": "1", "name": "Garry's Mod", "executables": [
                {"name": ">hl2.exe", "os": host_os(), "arguments": "-game garrysmod"}
            ]},
            {"id": "2", "name": "Half-Life 2", "executables": [
                {"name": ">hl2.exe", "os": host_os(), "arguments": "-game hl2"}
            ]},
            {"id": "3", "name": "Bare", "executables": [
                {"name": "bare.exe", "os": host_os()}
            ]}
        ]));
        let game = catalog
            .matches("/games/hl2.exe", || {
                "HL2.EXE -Game GARRYSMOD -windowed".into()
            })
            .expect("arguments match");
        assert_eq!(game.id, 1);
        assert!(
            catalog
                .matches("/games/hl2.exe", || "-game".into())
                .is_none()
        );
        assert!(
            catalog
                .matches("/games/bare.exe", || panic!("command line was read"))
                .is_some()
        );
    }

    #[test]
    fn cached_catalogs_restore_their_rules() {
        let catalog = catalog(json!([
            {"id": "1", "name": "Game", "executables": [
                {"name": ">game.exe", "os": host_os(), "arguments": "-play"},
                {"name": ">dir/exact.exe", "os": host_os()}
            ]}
        ]));
        let bytes = serde_json::to_vec(&catalog.cached()).expect("cache serializes");
        let restored = decode_cached(&bytes).expect("cache decodes");
        assert!(
            restored
                .matches("/opt/game.exe", || "-play".into())
                .is_some()
        );
        assert!(restored.matches("/opt/game.exe", String::new).is_none());
        assert!(restored.matches("/opt/exact.exe", String::new).is_none());
        assert!(
            restored
                .matches("/opt/dir/exact.exe", String::new)
                .is_none()
        );
    }

    #[test]
    fn cache_files_round_trip() {
        let directory = std::env::temp_dir().join(format!(
            "zcord-catalog-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let path = directory.join("detectable-games.json");
        let catalog = catalog(json!([
            {"id": "1", "name": "Game", "executables": [
                {"name": "game.exe", "os": host_os()}
            ]}
        ]));
        write_cache(&path, &catalog).expect("cache writes");
        let (restored, age) = read_cache(&path).expect("cache reads");
        assert!(age < MAX_AGE);
        assert!(restored.matches("/opt/game.exe", String::new).is_some());
        let _ = fs::remove_dir_all(&directory);
    }
}
