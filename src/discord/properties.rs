use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

pub const LOCALE: &str = "en-US";
pub fn timezone() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "Etc/UTC".into())
}
pub const ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";
pub const SEC_CH_UA: &str = "\"Not?A_Brand\";v=\"24\", \"Chromium\";v=\"152\"";

const CHROME_VERSION: &str = "152.0.7977.167";
const ELECTRON_VERSION: &str = "44.9.3";
const CLIENT_BUILD_NUMBER: u32 = 660103;
const NATIVE_BUILD_NUMBER: u32 = 97026;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct IdentityCache {
    build_number: Option<u32>,
    client_versions: HashMap<String, String>,
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

fn identity() -> &'static Identity {
    // Keep REST, media, and gateway versions fixed together until the next launch.
    static IDENTITY: OnceLock<Identity> = OnceLock::new();
    IDENTITY.get_or_init(|| {
        let cache = crate::settings::discord_identity();
        let fallback = if cfg!(target_os = "linux") {
            "1.0.168"
        } else if cfg!(target_os = "macos") {
            "0.0.421"
        } else {
            "1.0.9266"
        };
        let client_version = if cfg!(target_os = "windows") {
            fallback.to_owned()
        } else {
            cache.client_versions.get(&version_key())
                .filter(|version| valid_client_version(version))
                .cloned().unwrap_or_else(|| fallback.to_owned())
        };
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
        let user_agent = format!("Mozilla/5.0 ({platform}) AppleWebKit/537.36 (KHTML, like Gecko) discord/{client_version} Chrome/{CHROME_VERSION} Electron/{ELECTRON_VERSION} Safari/537.36");
        Identity {
            build_number: cache.build_number.filter(|number| *number > 0).unwrap_or(CLIENT_BUILD_NUMBER),
            client_version,
            user_agent,
        }
    })
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

pub(crate) async fn refresh_identity(mut cache: IdentityCache) -> IdentityCache {
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
    // The legacy Windows endpoint still advertises a 2021 build.
    let platform = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        return cache;
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

struct Platform {
    version: String,
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
        Platform {
            version: if cfg!(target_os = "windows") {
                "10.0.26300".to_owned()
            } else {
                sysinfo::System::kernel_version().unwrap_or_else(|| "unknown".to_owned())
            },
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
    os_arch: &'static str,
    app_arch: &'static str,
    system_locale: &'static str,
    has_client_mods: bool,
    client_launch_id: &'static str,
    browser_user_agent: &'static str,
    browser_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    os_sdk_version: Option<&'static str>,
    client_build_number: u32,
    native_build_number: u32,
    client_event_source: Option<()>,
    launch_signature: &'static str,
    client_heartbeat_session_id: String,
    client_app_state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    window_manager: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    distro: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_environment: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_server: Option<&'static str>,
}

struct Launch {
    id: String,
    signature: String,
    heartbeat: Mutex<Heartbeat>,
}

struct Heartbeat {
    session: String,
    started: Instant,
}

impl Heartbeat {
    fn new(now: Instant) -> Self {
        Self {
            session: uuid_v4(),
            started: now,
        }
    }

    fn session_at(&mut self, now: Instant) -> &str {
        if now.saturating_duration_since(self.started) >= HEARTBEAT_INTERVAL {
            *self = Self::new(now);
        }
        &self.session
    }
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
        os_arch: &platform.os_arch,
        app_arch: app_arch(),
        system_locale: LOCALE,
        has_client_mods: false,
        client_launch_id: &launch.id,
        browser_user_agent: user_agent(),
        browser_version: ELECTRON_VERSION,
        os_sdk_version: cfg!(target_os = "windows").then_some("26300"),
        client_build_number: identity.build_number,
        native_build_number: NATIVE_BUILD_NUMBER,
        client_event_source: None,
        launch_signature: &launch.signature,
        client_heartbeat_session_id: heartbeat,
        client_app_state: "focused",
        window_manager: platform.window_manager.as_deref(),
        distro: platform.distro.as_deref(),
        runtime_environment: platform.runtime_environment,
        display_server: platform.display_server.as_deref(),
    }
}

pub fn client_properties() -> Value {
    serde_json::to_value(properties()).unwrap_or_default()
}

pub fn super_properties() -> String {
    STANDARD.encode(serde_json::to_string(&properties()).unwrap_or_default())
}

pub fn identify(token: &str) -> Value {
    json!({"op": 2, "d": {
        "token": token, "capabilities": CAPABILITIES, "properties": client_properties(),
        "compress": false, "presence": {"status": "unknown", "activities": [], "since": 0, "afk": false},
        "client_state": {"guild_versions": {}, "api_code_version": 0}
    }})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identify_is_user_identify_and_properties_match_rest() {
        let payload = identify("fixture-secret");
        assert_eq!(payload["d"]["capabilities"], CAPABILITIES);
        assert!(payload["d"]["client_state"].is_object());
        assert_eq!(payload["d"]["presence"]["status"], "unknown");
        assert!(!payload.to_string().contains("Bot "));
        let raw = STANDARD.decode(super_properties()).unwrap();
        let decoded: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(payload["d"]["properties"], decoded);
        let raw = String::from_utf8(raw).unwrap();
        let expected_os = if cfg!(target_os = "linux") {
            "Linux"
        } else if cfg!(target_os = "macos") {
            "Mac OS X"
        } else {
            "Windows"
        };
        assert!(raw.starts_with(&format!(r#"{{"os":"{expected_os}","browser":"Discord Client","release_channel":"stable","client_version":"{}","os_version":"{}""#, identity().client_version, platform().version)));
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
            assert_eq!(decoded["os_sdk_version"], "26300");
            assert_eq!(identity().client_version, "1.0.9266");
            if app_arch() == "x64" {
                assert_eq!(
                    user_agent(),
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) discord/1.0.9266 Chrome/152.0.7977.167 Electron/44.9.3 Safari/537.36"
                );
            }
        } else {
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
    fn heartbeat_rotates_at_thirty_minutes_without_changing_launch_ids() {
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
        assert_eq!(
            heartbeat.session_at(now + HEARTBEAT_INTERVAL - Duration::from_nanos(1)),
            first
        );
        let second = heartbeat.session_at(now + HEARTBEAT_INTERVAL).to_owned();
        assert_ne!(second, first);
        assert_eq!(heartbeat.session_at(now + HEARTBEAT_INTERVAL), second);
        assert_eq!(
            heartbeat.session_at(now + HEARTBEAT_INTERVAL * 2 - Duration::from_nanos(1)),
            second
        );
        assert_ne!(heartbeat.session_at(now + HEARTBEAT_INTERVAL * 2), second);
        let after_idle = heartbeat
            .session_at(now + HEARTBEAT_INTERVAL * 10)
            .to_owned();
        assert_eq!(
            heartbeat.session_at(now + HEARTBEAT_INTERVAL * 10),
            after_idle
        );
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
