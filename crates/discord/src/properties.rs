use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Mutex, OnceLock, PoisonError, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const LOCALE: &str = "en-US";
pub fn timezone() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "Etc/UTC".into())
}
pub const ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";
pub const SEC_CH_UA: &str = "\"Not?A_Brand\";v=\"24\", \"Chromium\";v=\"152\"";

const CHROME_VERSION: &str = "152.0.7977.169";
const ELECTRON_VERSION: &str = "44.10.2";
const CLIENT_BUILD_NUMBER: u32 = 673919;
const NATIVE_BUILD_NUMBER: u32 = 98625;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IdentityCache {
    build_number: Option<u32>,
    client_versions: HashMap<String, String>,
    installation_id: Option<String>,
}

impl IdentityCache {
    pub fn absorb(&mut self, refreshed: IdentityCache) {
        self.build_number = refreshed.build_number;
        self.client_versions = refreshed.client_versions;
    }

    pub fn installation_id(&self) -> Option<&str> {
        self.installation_id.as_deref()
    }

    pub fn set_installation_id(&mut self, id: &str) {
        self.installation_id = Some(id.to_owned());
    }
}

struct Identity {
    build_number: u32,
    client_version: String,
    user_agent: String,
}

fn app_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        arch => arch,
    }
}

fn version_key() -> String {
    format!("{}-{}", std::env::consts::OS, app_arch())
}

fn valid_client_version(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && part.parse::<u32>().is_ok()
        })
}

static IDENTITY_SOURCE: OnceLock<fn() -> IdentityCache> = OnceLock::new();

pub fn set_identity_source(load: fn() -> IdentityCache) {
    let _ = IDENTITY_SOURCE.set(load);
}

static IDENTITY: RwLock<Option<&'static Identity>> = RwLock::new(None);

fn identity() -> &'static Identity {
    if let Some(identity) = *IDENTITY.read().unwrap_or_else(PoisonError::into_inner) {
        return identity;
    }
    let mut slot = IDENTITY.write().unwrap_or_else(PoisonError::into_inner);
    slot.get_or_insert_with(|| {
        let cache = IDENTITY_SOURCE.get().map(|load| load()).unwrap_or_default();
        seed_installation_id(&cache);
        Box::leak(Box::new(build_identity(&cache)))
    })
}

/// Leaks one small allocation per refresh so callers can keep borrowing `&'static str`s.
pub fn apply_identity(cache: &IdentityCache) {
    seed_installation_id(cache);
    *IDENTITY.write().unwrap_or_else(PoisonError::into_inner) =
        Some(Box::leak(Box::new(build_identity(cache))));
}

fn seed_installation_id(cache: &IdentityCache) {
    let mut installation = INSTALLATION_ID
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if installation.is_none() {
        installation.clone_from(&cache.installation_id);
    }
}

fn build_identity(cache: &IdentityCache) -> Identity {
    {
        let fallback = if cfg!(target_os = "linux") {
            "1.0.168"
        } else if cfg!(target_os = "macos") {
            "0.0.421"
        } else {
            "1.0.9269"
        };
        let client_version = cache
            .client_versions
            .get(&version_key())
            .filter(|version| valid_client_version(version))
            .cloned()
            .unwrap_or_else(|| fallback.to_owned());
        let platform = if cfg!(target_os = "linux") {
            match app_arch() {
                "arm64" => "X11; Linux aarch64",
                "ia32" => "X11; Linux i686",
                _ => "X11; Linux x86_64",
            }
        } else if cfg!(target_os = "macos") {
            "Macintosh; Intel Mac OS X 10_15_7"
        } else if app_arch() == "arm64" {
            "Windows NT 10.0; Win64; arm64"
        } else if app_arch() == "ia32" {
            "Windows NT 10.0; Win32; x86"
        } else {
            "Windows NT 10.0; Win64; x64"
        };
        let user_agent = format!(
            "Mozilla/5.0 ({platform}) AppleWebKit/537.36 (KHTML, like Gecko) discord/{client_version} Chrome/{CHROME_VERSION} Electron/{ELECTRON_VERSION} Safari/537.36"
        );
        Identity {
            build_number: cache
                .build_number
                .filter(|number| *number > 0)
                .unwrap_or(CLIENT_BUILD_NUMBER),
            client_version,
            user_agent,
        }
    }
}

pub fn user_agent() -> &'static str {
    &identity().user_agent
}

pub(super) fn sec_ch_ua_platform() -> &'static str {
    if cfg!(target_os = "linux") {
        "\"Linux\""
    } else if cfg!(target_os = "macos") {
        "\"macOS\""
    } else {
        "\"Windows\""
    }
}

fn parse_build_number(html: &str) -> Option<u32> {
    let (_, env) = html.split_once("window.GLOBAL_ENV")?;
    let env = env.split("</script>").next()?;
    let (_, value) = env.split_once("\"BUILD_NUMBER\"")?;
    let value = value.trim_start().strip_prefix(':')?.trim_start();
    let literal = serde_json::Deserializer::from_str(value)
        .into_iter::<Value>()
        .next()?
        .ok()?;
    let number = match literal {
        Value::String(digits)
            if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            digits.parse::<u32>().ok()?
        }
        Value::Number(number) => u32::try_from(number.as_u64()?).ok()?,
        _ => return None,
    };
    (number > 0).then_some(number)
}

pub async fn refresh_identity(mut cache: IdentityCache) -> IdentityCache {
    let Ok(client) = reqwest::Client::builder()
        .user_agent(user_agent())
        .timeout(Duration::from_secs(5))
        .build()
    else {
        return cache;
    };
    if let Ok(response) = client.get("https://discord.com/app").send().await
        && let Ok(response) = response.error_for_status()
        && let Ok(html) = response.text().await
        && let Some(number) = parse_build_number(&html)
    {
        cache.build_number = Some(number);
    }
    if cfg!(target_os = "windows") {
        if let Some(version) = windows_host_version(&client).await {
            cache.client_versions.insert(version_key(), version);
        }
        return cache;
    }
    let platform = if cfg!(target_os = "linux") {
        "linux"
    } else {
        "osx"
    };
    if let Ok(response) = client
        .get("https://discord.com/api/updates/stable")
        .query(&[("platform", platform), ("arch", app_arch())])
        .send()
        .await
        && let Ok(response) = response.error_for_status()
        && let Ok(value) = response.json::<Value>().await
        && let Some(version) = value.get("name").and_then(Value::as_str)
        && valid_client_version(version)
    {
        cache
            .client_versions
            .insert(version_key(), version.to_owned());
    }
    cache
}

// The legacy /api/updates endpoint still advertises a 2021 Windows build.
async fn windows_host_version(client: &reqwest::Client) -> Option<String> {
    let arch = match app_arch() {
        "ia32" => "x86",
        arch => arch,
    };
    let manifest = client
        .get("https://updates.discord.com/distributions/app/manifests/latest")
        .query(&[("channel", "stable"), ("platform", "win"), ("arch", arch)])
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<Value>()
        .await
        .ok()?;
    host_version(&manifest)
}

fn host_version(manifest: &Value) -> Option<String> {
    let parts = manifest.get("full")?.get("host_version")?.as_array()?;
    let version = parts
        .iter()
        .map(|part| part.as_u64().map(|part| part.to_string()))
        .collect::<Option<Vec<_>>>()?
        .join(".");
    valid_client_version(&version).then_some(version)
}

struct Platform {
    version: String,
    sdk_version: Option<String>,
    build_revision: Option<String>,
    os_arch: String,
    window_manager: Option<String>,
    distro: Option<String>,
    runtime_environment: Option<&'static str>,
    display_server: Option<String>,
}

fn platform() -> &'static Platform {
    static PLATFORM: OnceLock<Platform> = OnceLock::new();
    PLATFORM.get_or_init(|| {
        let linux = cfg!(target_os = "linux");
        let env = |key| std::env::var(key).unwrap_or_else(|_| "unknown".to_owned());
        let os_arch =
            sysinfo::System::cpu_arch().unwrap_or_else(|| std::env::consts::ARCH.to_owned());
        let kernel = sysinfo::System::kernel_version().unwrap_or_else(|| "unknown".to_owned());
        let windows = windows_build();
        Platform {
            version: if cfg!(target_os = "windows") {
                format!("10.0.{}", windows.0.as_deref().unwrap_or("26100"))
            } else if cfg!(target_os = "macos") {
                sysinfo::System::os_version().unwrap_or_else(|| kernel.clone())
            } else {
                kernel.clone()
            },
            sdk_version: if cfg!(target_os = "windows") {
                Some(windows.0.unwrap_or_else(|| "26100".to_owned()))
            } else if cfg!(target_os = "macos") {
                kernel.split('.').next().map(str::to_owned)
            } else {
                None
            },
            build_revision: windows.1,
            os_arch: match os_arch.as_str() {
                "x86_64" | "amd64" => "x64".to_owned(),
                "aarch64" => "arm64".to_owned(),
                "x86" | "i686" => "ia32".to_owned(),
                _ => os_arch,
            },
            window_manager: linux
                .then(|| format!("{},{}", env("XDG_CURRENT_DESKTOP"), env("GDMSESSION"))),
            distro: linux
                .then(|| {
                    std::process::Command::new("lsb_release")
                        .arg("-ds")
                        .output()
                        .ok()
                })
                .flatten()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            runtime_environment: linux.then(|| {
                if std::env::var_os("FLATPAK_ID").is_some() {
                    "flatpak"
                } else if std::env::var_os("SNAP").is_some() {
                    "snap"
                } else if std::env::var_os("APPIMAGE").is_some() {
                    "appimage"
                } else {
                    "native"
                }
            }),
            display_server: linux.then(|| env("XDG_SESSION_TYPE")),
        }
    })
}

#[cfg(windows)]
fn windows_build() -> (Option<String>, Option<String>) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(output) = std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return (None, None);
    };
    parse_windows_build(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(windows))]
fn windows_build() -> (Option<String>, Option<String>) {
    (None, None)
}

#[cfg(any(windows, test))]
fn parse_windows_build(output: &str) -> (Option<String>, Option<String>) {
    let value = |name: &str| {
        output.lines().find_map(|line| {
            let mut fields = line.split_whitespace();
            (fields.next()? == name).then(|| fields.nth(1).map(str::to_owned))?
        })
    };
    let build = value("CurrentBuildNumber")
        .filter(|build| !build.is_empty() && build.bytes().all(|byte| byte.is_ascii_digit()));
    let revision = value("UBR")
        .and_then(|ubr| u64::from_str_radix(ubr.trim_start_matches("0x"), 16).ok())
        .map(|ubr| ubr.to_string());
    (build, revision)
}

// https://docs.discord.food/gateway/gateway-events#example-identify
// 1734653 enables DEDUPE_USER_OBJECTS, PRIORITIZED_READY_PAYLOAD, VERSIONED_READ_STATES,
// and CLIENT_STATE_V2: users live in `users`, own members in guild-indexed `merged_members`,
// read_state is {entries,version,partial}, and guild metadata lives in `properties`.
// READY_SUPPLEMENTAL supplies other members/presences; browsing sends op 37 subscriptions
// and targeted op 8 member requests, never presence updates.
pub const CAPABILITIES: u64 = 1_734_653;

#[derive(Serialize)]
struct ClientProperties {
    os: &'static str,
    browser: &'static str,
    release_channel: &'static str,
    client_version: &'static str,
    os_version: &'static str,
    os_version_is_release: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    os_sdk_version: Option<&'static str>,
    os_arch: &'static str,
    app_arch: &'static str,
    system_locale: &'static str,
    has_client_mods: bool,
    client_launch_id: &'static str,
    browser_user_agent: &'static str,
    browser_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    window_manager: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    distro: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_environment: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_server: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    os_build_revision: Option<&'static str>,
    client_build_number: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_build_number: Option<u32>,
    client_event_source: Option<()>,
    launch_signature: &'static str,
    client_heartbeat_session_id: String,
    client_app_state: &'static str,
}

static FOCUSED: AtomicBool = AtomicBool::new(true);
static INSTALLATION_ID: Mutex<Option<String>> = Mutex::new(None);
static IDENTIFIED: AtomicBool = AtomicBool::new(false);

pub fn set_focused(focused: bool) {
    FOCUSED.store(focused, Ordering::Relaxed);
}

pub fn installation_id() -> Option<String> {
    identity();
    INSTALLATION_ID
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub fn set_installation_id(id: Option<String>) {
    *INSTALLATION_ID
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = id.filter(|id| !id.is_empty());
}

struct Launch {
    id: String,
    signature: String,
    heartbeat: Mutex<Heartbeat>,
}

struct Heartbeat {
    session: String,
    created_ms: u64,
    last_used: Instant,
}

impl Heartbeat {
    fn new(now: Instant) -> Self {
        Self {
            session: uuid_v4(),
            created_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_millis() as u64),
            last_used: now,
        }
    }

    fn session_at(&mut self, now: Instant) -> &str {
        if now.saturating_duration_since(self.last_used) >= HEARTBEAT_INTERVAL {
            *self = Self::new(now);
        }
        self.last_used = now;
        &self.session
    }
}

/// Reading does not count as a use, so it never postpones the idle rotation.
pub fn heartbeat_session() -> (String, u64) {
    let heartbeat = launch()
        .heartbeat
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    (heartbeat.session.clone(), heartbeat.created_ms)
}

pub fn launch_id() -> &'static str {
    &launch().id
}

fn launch() -> &'static Launch {
    static LAUNCH: OnceLock<Launch> = OnceLock::new();
    LAUNCH.get_or_init(|| Launch {
        id: uuid_v4(),
        signature: launch_signature(),
        heartbeat: Mutex::new(Heartbeat::new(Instant::now())),
    })
}

pub(super) fn uuid_v4() -> String {
    let mut bytes: [u8; 16] = rand::random();
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    format_uuid(bytes)
}

// https://docs.discord.food/reference#launch-signature: these bits flag client mods.
const MOD_BITS: u128 = (1 << 119)
    | (1 << 108)
    | (1 << 100)
    | (1 << 91)
    | (1 << 84)
    | (1 << 75)
    | (1 << 61)
    | (1 << 55)
    | (1 << 48)
    | (1 << 38)
    | (1 << 24)
    | (1 << 11);

fn launch_signature() -> String {
    let mut bytes: [u8; 16] = rand::random();
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    format_uuid((u128::from_be_bytes(bytes) & !MOD_BITS).to_be_bytes())
}

fn format_uuid(bytes: [u8; 16]) -> String {
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn properties() -> ClientProperties {
    let launch = launch();
    let identity = identity();
    let platform = platform();
    let heartbeat = launch
        .heartbeat
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .session_at(Instant::now())
        .to_owned();
    ClientProperties {
        os: if cfg!(target_os = "linux") {
            "Linux"
        } else if cfg!(target_os = "macos") {
            "Mac OS X"
        } else {
            "Windows"
        },
        browser: "Discord Client",
        release_channel: "stable",
        client_version: &identity.client_version,
        os_version: &platform.version,
        os_version_is_release: true,
        os_sdk_version: platform.sdk_version.as_deref(),
        os_arch: &platform.os_arch,
        app_arch: app_arch(),
        system_locale: LOCALE,
        has_client_mods: false,
        client_launch_id: &launch.id,
        browser_user_agent: user_agent(),
        browser_version: ELECTRON_VERSION,
        window_manager: platform.window_manager.as_deref(),
        distro: platform.distro.as_deref(),
        runtime_environment: platform.runtime_environment,
        display_server: platform.display_server.as_deref(),
        os_build_revision: platform.build_revision.as_deref(),
        client_build_number: identity.build_number,
        native_build_number: cfg!(target_os = "windows").then_some(NATIVE_BUILD_NUMBER),
        client_event_source: None,
        launch_signature: &launch.signature,
        client_heartbeat_session_id: heartbeat,
        client_app_state: if FOCUSED.load(Ordering::Relaxed) {
            "focused"
        } else {
            "unfocused"
        },
    }
}

pub fn client_properties() -> Value {
    serde_json::to_value(properties()).unwrap_or_default()
}

pub fn super_properties() -> String {
    STANDARD.encode(serde_json::to_string(&properties()).unwrap_or_default())
}

pub fn identify(token: &str, active: bool) -> Value {
    let mut properties = client_properties();
    let fast_connect = !IDENTIFIED.swap(true, Ordering::Relaxed);
    properties["is_fast_connect"] = json!(fast_connect);
    if !fast_connect {
        properties["gateway_connect_reasons"] = json!("NO_REASONS");
    }
    if let Some(installation_id) = installation_id() {
        properties["installation_id"] = json!(installation_id);
    }
    if fast_connect {
        return json!({"op": 2, "d": {
            "token": token, "capabilities": CAPABILITIES, "properties": properties,
            "client_state": {"guild_versions": {}}
        }});
    }
    json!({"op": 2, "d": {
        "token": token, "capabilities": CAPABILITIES, "properties": properties,
        "presence": {"status": "unknown", "since": 0, "activities": [], "afk": false},
        "compress": false, "client_state": {"guild_versions": {}},
        "qos_token": if active { "CgIIAQ==" } else { "CgA=" }
    }})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identify_is_user_identify_and_properties_match_rest() {
        identify("fixture-secret", true);
        let payload = identify("fixture-secret", false);
        assert_eq!(payload["d"]["capabilities"], CAPABILITIES);
        assert_eq!(payload["d"]["client_state"], json!({"guild_versions": {}}));
        assert_eq!(payload["d"]["presence"]["status"], "unknown");
        assert_eq!(payload["d"]["compress"], false);
        assert_eq!(payload["d"]["qos_token"], "CgA=");
        assert!(!payload.to_string().contains("Bot "));
        let mut properties = payload["d"]["properties"].clone();
        let extras = properties.as_object_mut().unwrap();
        assert_eq!(extras.remove("is_fast_connect"), Some(json!(false)));
        assert_eq!(
            extras.remove("gateway_connect_reasons"),
            Some(json!("NO_REASONS"))
        );
        extras.remove("installation_id");
        let raw = STANDARD.decode(super_properties()).unwrap();
        let decoded: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(properties, decoded);
        assert_eq!(decoded["os_version_is_release"], true);
        let raw = String::from_utf8(raw).unwrap();
        let expected_os = if cfg!(target_os = "linux") {
            "Linux"
        } else if cfg!(target_os = "macos") {
            "Mac OS X"
        } else {
            "Windows"
        };
        assert!(raw.starts_with(&format!(r#"{{"os":"{expected_os}","browser":"Discord Client","release_channel":"stable","client_version":"{}","os_version":"{}","os_version_is_release":true,"#, identity().client_version, platform().version)));
        assert_eq!(decoded["browser_user_agent"], user_agent());
        assert!(user_agent().contains(&format!(
            "discord/{} ",
            decoded["client_version"].as_str().unwrap()
        )));
        assert_eq!(decoded["app_arch"], app_arch());
        assert_eq!(decoded["os_arch"], platform().os_arch);
        assert_eq!(decoded["client_build_number"], identity().build_number);
        if cfg!(target_os = "windows") {
            assert!(raw.ends_with(r#""client_app_state":"focused"}"#));
            assert_eq!(decoded["native_build_number"], NATIVE_BUILD_NUMBER);
            assert_eq!(identity().client_version, "1.0.9269");
            if app_arch() == "x64" {
                assert_eq!(
                    user_agent(),
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) discord/1.0.9269 Chrome/152.0.7977.169 Electron/44.10.2 Safari/537.36"
                );
            }
        } else {
            assert!(decoded.get("native_build_number").is_none());
        }
        if cfg!(target_os = "linux") {
            assert!(decoded.get("os_sdk_version").is_none());
            assert_eq!(
                decoded["os_version"],
                sysinfo::System::kernel_version().unwrap()
            );
        }
        assert_eq!(
            decoded.get("window_manager").is_some(),
            cfg!(target_os = "linux")
        );
        assert_eq!(
            decoded.get("display_server").is_some(),
            cfg!(target_os = "linux")
        );
        assert_eq!(
            decoded.get("runtime_environment").is_some(),
            cfg!(target_os = "linux")
        );
        let launch_id = decoded["client_launch_id"].as_str().unwrap();
        assert_eq!(launch_id.len(), 36);
        assert_eq!(&launch_id[14..15], "4");
        assert_eq!(super_properties(), super_properties());
    }

    #[test]
    fn host_version_and_windows_build_parse() {
        let manifest = json!({"full": {"host_version": [1, 0, 9269]}});
        assert_eq!(host_version(&manifest).as_deref(), Some("1.0.9269"));
        assert_eq!(
            host_version(&json!({"full": {"host_version": [1, "0"]}})),
            None
        );
        let output = "\r\nHKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\r\n    CurrentBuild    REG_SZ    26300\r\n    CurrentBuildNumber    REG_SZ    26300\r\n    UBR    REG_DWORD    0x26f6\r\n";
        assert_eq!(
            parse_windows_build(output),
            (Some("26300".to_owned()), Some("9974".to_owned()))
        );
        assert_eq!(parse_windows_build(""), (None, None));
    }

    #[test]
    fn build_number_parses_global_env_string_and_number() {
        for html in [
            r#"<script>window.GLOBAL_ENV={"HTML_TIMESTAMP":Date.now(),"BUILD_NUMBER":"660103","PROJECT_ENV":"production"};</script>"#,
            "window.GLOBAL_ENV = {\n\"BUILD_NUMBER\" : \"660103\"\n};",
            r#"window.GLOBAL_ENV={"BUILD_NUMBER":660103};"#,
        ] {
            assert_eq!(parse_build_number(html), Some(660103));
        }
    }

    #[test]
    fn build_number_rejects_missing_invalid_and_overflow_values() {
        for html in [
            "",
            r#"{"BUILD_NUMBER":"660103"}"#,
            r#"window.GLOBAL_ENV={};</script><script>{"BUILD_NUMBER":"660103"}</script>"#,
            r#"window.GLOBAL_ENV={"BUILD_NUMBER":""};"#,
            r#"window.GLOBAL_ENV={"BUILD_NUMBER":"0"};"#,
            r#"window.GLOBAL_ENV={"BUILD_NUMBER":"-1"};"#,
            r#"window.GLOBAL_ENV={"BUILD_NUMBER":"660103x"};"#,
            r#"window.GLOBAL_ENV={"BUILD_NUMBER":"4294967296"};"#,
            r#"window.GLOBAL_ENV={"BUILD_NUMBER":null};"#,
            r#"window.GLOBAL_ENV={"BUILD_NUMBER"="660103"};"#,
            r#"window.GLOBAL_ENV={"BUILD_NUMBER":"660103};"#,
        ] {
            assert_eq!(parse_build_number(html), None, "{html}");
        }
    }

    #[test]
    fn heartbeat_rotates_after_thirty_idle_minutes_without_changing_launch_ids() {
        let now = Instant::now();
        let launch = Launch {
            id: uuid_v4(),
            signature: launch_signature(),
            heartbeat: Mutex::new(Heartbeat::new(now)),
        };
        let launch_id = launch.id.clone();
        let signature = launch.signature.clone();
        let mut heartbeat = launch.heartbeat.lock().unwrap();
        let first = heartbeat.session_at(now).to_owned();
        let almost = now + HEARTBEAT_INTERVAL - Duration::from_nanos(1);
        assert_eq!(heartbeat.session_at(almost), first);
        assert_eq!(heartbeat.session_at(now + HEARTBEAT_INTERVAL), first);
        let idle = almost + HEARTBEAT_INTERVAL;
        let second = heartbeat.session_at(idle).to_owned();
        assert_ne!(second, first);
        assert_eq!(heartbeat.session_at(idle), second);
        assert_eq!(launch.id, launch_id);
        assert_eq!(launch.signature, signature);
    }

    #[test]
    fn identity_cache_round_trips_and_defaults_missing_fields() {
        let mut cache: IdentityCache = serde_json::from_str("{}").unwrap();
        assert_eq!(cache.build_number, None);
        cache.build_number = Some(660103);
        cache
            .client_versions
            .insert("linux-x64".into(), "1.0.168".into());
        let decoded: IdentityCache =
            serde_json::from_str(&serde_json::to_string(&cache).unwrap()).unwrap();
        assert_eq!(decoded.build_number, cache.build_number);
        assert_eq!(decoded.client_versions, cache.client_versions);
        for version in ["1.0.168", "0.0.421", "1.0.9264"] {
            assert!(valid_client_version(version));
        }
        for version in ["", "1.2", "1.2.3.4", "1.2.x", "1.2.3\r\n", "1.2.4294967296"] {
            assert!(!valid_client_version(version));
        }
    }

    #[test]
    fn launch_signature_clears_mod_bits_and_keeps_uuid_v4_bits() {
        for _ in 0..1000 {
            let signature = launch_signature();
            assert_eq!(signature.len(), 36);
            let value = u128::from_str_radix(&signature.replace('-', ""), 16).unwrap();
            assert_eq!(value & MOD_BITS, 0);
            assert_eq!((value >> 76) & 0xF, 4);
            assert_eq!((value >> 62) & 0b11, 0b10);
        }
    }
}
