use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

pub const LOCALE: &str = "en-US";
pub fn timezone() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "Etc/UTC".into())
}
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) discord/1.0.328 Chrome/134.0.6998.179 Electron/35.1.5 Safari/537.36";

// Documented desktop example, not a claim to track the latest release. Refresh manually from
// Discord Settings > What's New / the build shown at the bottom of Settings (never scrape assets).
const CLIENT_BUILD_NUMBER: u32 = 397417;

// https://docs.discord.food/gateway/gateway-events#example-identify
// 1734653 enables DEDUPE_USER_OBJECTS, PRIORITIZED_READY_PAYLOAD, VERSIONED_READ_STATES,
// and CLIENT_STATE_V2: users live in `users`, own members in guild-indexed `merged_members`,
// read_state is {entries,version,partial}, and guild metadata lives in `properties`.
// READY_SUPPLEMENTAL is deliberately ignored; no subscriptions or lazy guild requests are sent.
pub const CAPABILITIES: u64 = 1_734_653;

pub fn client_properties() -> Value {
    json!({
        "os": "Windows", "browser": "Discord Client", "release_channel": "stable",
        "client_version": "1.0.328", "os_version": "10.0.26100", "os_arch": "x64",
        "app_arch": "x64", "system_locale": LOCALE, "browser_user_agent": USER_AGENT,
        "browser_version": "35.1.5", "client_build_number": CLIENT_BUILD_NUMBER,
        "os_sdk_version": "26100", "client_event_source": null
    })
}

pub fn super_properties() -> String {
    STANDARD.encode(client_properties().to_string())
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
        let decoded: Value =
            serde_json::from_slice(&STANDARD.decode(super_properties()).unwrap()).unwrap();
        assert_eq!(payload["d"]["properties"], decoded);
    }
}
