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

const URL: &str = "https://cdn.discordapp.com/detectables/games-v1.json";
const MAX_BODY: usize = 32 * 1024 * 1024;
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY: Duration = Duration::from_secs(60 * 60);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
const CACHE_FILE: &str = "detectable-games.json";
const ETAG_FILE: &str = "detectable-games.etag";

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

fn etag_path(cache: &Path) -> PathBuf {
    cache.with_file_name(ETAG_FILE)
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

fn write_cache(path: &Path, catalog: &Catalog, etag: Option<&str>) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    let etag_file = etag_path(path);
    let _ = fs::remove_file(&etag_file);
    let bytes = serde_json::to_vec(&catalog.cached()).map_err(std::io::Error::other)?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)?;
    match etag {
        Some(etag) => fs::write(etag_file, etag),
        None => Ok(()),
    }
}

fn touch_cache(path: &Path) -> std::io::Result<()> {
    fs::File::options()
        .write(true)
        .open(path)?
        .set_modified(SystemTime::now())
}

enum Download {
    Modified { body: Vec<u8>, etag: Option<String> },
    NotModified,
}

async fn download(client: &reqwest::Client, etag: Option<&str>) -> Result<Download, String> {
    let mut request = client.get(URL);
    if let Some(etag) = etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let mut response = request.send().await.map_err(|error| error.to_string())?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED && etag.is_some() {
        return Ok(Download::NotModified);
    }
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|etag| etag.to_str().ok())
        .map(str::to_owned);
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
    Ok(Download::Modified { body, etag })
}

pub(crate) async fn run(output: watch::Sender<Option<Arc<Catalog>>>) {
    let client = reqwest::Client::builder()
        .user_agent(crate::discord::user_agent())
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .unwrap_or_default();
    let path = cache_path();
    loop {
        let mut etag = None;
        if let Some(path) = &path
            && let Some((catalog, age)) = read_cache(path)
        {
            if output.borrow().is_none() {
                output.send_replace(Some(Arc::new(catalog)));
            }
            if age < MAX_AGE {
                tokio::time::sleep(MAX_AGE - age).await;
            }
            etag = fs::read_to_string(etag_path(path)).ok();
        }
        let (body, etag) = match download(&client, etag.as_deref()).await {
            Ok(Download::Modified { body, etag }) => (body, etag),
            Ok(Download::NotModified) => {
                if let Some(path) = &path
                    && let Err(error) = touch_cache(path)
                {
                    log::warn!("Could not refresh the detectable game list cache: {error}");
                    let _ = fs::remove_file(etag_path(path));
                }
                tokio::time::sleep(MAX_AGE).await;
                continue;
            }
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
        if let Some(path) = &path
            && let Err(error) = write_cache(path, &catalog, etag.as_deref())
        {
            log::warn!("Could not cache the detectable game list: {error}");
        }
        output.send_replace(Some(Arc::new(catalog)));
        tokio::time::sleep(MAX_AGE).await;
    }
}

#[cfg(test)]
mod tests;
