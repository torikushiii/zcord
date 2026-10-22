use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::OnceLock;

pub const LOCALE: &str = "en-US";
pub fn timezone() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "Etc/UTC".into())
}
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) discord/1.0.9264 Chrome/148.0.7778.280 Electron/42.11.15 Safari/537.36";
pub const ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";
pub const SEC_CH_UA: &str = "\"Not/A)Brand\";v=\"99\", \"Chromium\";v=\"148\"";

const CLIENT_VERSION: &str = "1.0.9264";
const ELECTRON_VERSION: &str = "42.11.15";
const OS_VERSION: &str = "10.0.26300";
const OS_SDK_VERSION: &str = "26300";
const CLIENT_BUILD_NUMBER: u32 = 643186;
const NATIVE_BUILD_NUMBER: u32 = 96127;

// https://docs.discord.food/gateway/gateway-events#example-identify
// 1734653 enables DEDUPE_USER_OBJECTS, PRIORITIZED_READY_PAYLOAD, VERSIONED_READ_STATES,
// and CLIENT_STATE_V2: users live in `users`, own members in guild-indexed `merged_members`,
// read_state is {entries,version,partial}, and guild metadata lives in `properties`.
// READY_SUPPLEMENTAL supplies other members/presences; browsing sends op 37 subscriptions
// and targeted op 8 member requests, never presence updates or message acknowledgements.
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
    os_sdk_version: &'static str,
    client_build_number: u32,
    native_build_number: u32,
    client_event_source: Option<()>,
    launch_signature: &'static str,
    client_heartbeat_session_id: &'static str,
    client_app_state: &'static str,
}

struct Launch {
    id: String,
    signature: String,
    heartbeat_session: String,
}

fn launch() -> &'static Launch {
    static LAUNCH: OnceLock<Launch> = OnceLock::new();
    LAUNCH.get_or_init(|| Launch {
        id: uuid_v4(),
        signature: uuid_v4(),
        heartbeat_session: uuid_v4(),
    })
}

fn uuid_v4() -> String {
    let mut bytes: [u8; 16] = rand::random();
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
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
    ClientProperties {
        os: "Windows",
        browser: "Discord Client",
        release_channel: "stable",
        client_version: CLIENT_VERSION,
        os_version: OS_VERSION,
        os_arch: "x64",
        app_arch: "x64",
        system_locale: LOCALE,
        has_client_mods: false,
        client_launch_id: &launch.id,
        browser_user_agent: USER_AGENT,
        browser_version: ELECTRON_VERSION,
        os_sdk_version: OS_SDK_VERSION,
        client_build_number: CLIENT_BUILD_NUMBER,
        native_build_number: NATIVE_BUILD_NUMBER,
        client_event_source: None,
        launch_signature: &launch.signature,
        client_heartbeat_session_id: &launch.heartbeat_session,
        client_app_state: "focused",
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
        assert!(raw.starts_with(r#"{"os":"Windows","browser":"Discord Client","release_channel":"stable","client_version":"1.0.9264","os_version":"10.0.26300""#));
        assert!(raw.ends_with(r#""client_app_state":"focused"}"#));
        let launch_id = decoded["client_launch_id"].as_str().unwrap();
        assert_eq!(launch_id.len(), 36);
        assert_eq!(&launch_id[14..15], "4");
        assert_eq!(super_properties(), super_properties());
    }
}
