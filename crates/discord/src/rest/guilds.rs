use super::dms::{MuteConfig, mute_fields};
use super::*;
use chrono::{DateTime, Utc};

#[derive(Serialize)]
struct GuildMute {
    muted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    mute_config: Option<MuteConfig>,
}

#[derive(Serialize)]
struct GuildNotifications {
    message_notifications: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum GuildNotificationFlagPatch {
    SuppressEveryone(bool),
    SuppressRoles(bool),
    NotifyHighlights(u8),
    MuteScheduledEvents(bool),
    MobilePush(bool),
}

fn guild_referer(guild_id: u64) -> String {
    format!("https://discord.com/channels/{guild_id}")
}

fn guild_settings(client: &Client, guild_id: u64) -> RequestBuilder {
    client
        .patch(format!(
            "https://discord.com/api/v9/users/@me/guilds/{guild_id}/settings"
        ))
        .header(REFERER, guild_referer(guild_id))
}

fn set_guild_mute(
    client: &Client,
    guild_id: u64,
    mute: Option<i64>,
    now: DateTime<Utc>,
) -> RequestBuilder {
    let (muted, mute_config) = mute_fields(mute, now);
    guild_settings(client, guild_id).json(&GuildMute { muted, mute_config })
}

fn set_guild_notifications(
    client: &Client,
    guild_id: u64,
    level: model::NotificationLevel,
) -> RequestBuilder {
    let message_notifications = match level {
        model::NotificationLevel::All => 0,
        model::NotificationLevel::Mentions => 1,
        model::NotificationLevel::Nothing => 2,
        model::NotificationLevel::Inherit => {
            unreachable!("Guild notification levels must be explicit")
        }
    };
    guild_settings(client, guild_id).json(&GuildNotifications {
        message_notifications,
    })
}

fn set_guild_notification_flag(
    client: &Client,
    guild_id: u64,
    flag: model::GuildNotificationFlag,
    on: bool,
) -> RequestBuilder {
    let body = match flag {
        model::GuildNotificationFlag::SuppressEveryone => {
            GuildNotificationFlagPatch::SuppressEveryone(on)
        }
        model::GuildNotificationFlag::SuppressRoles => {
            GuildNotificationFlagPatch::SuppressRoles(on)
        }
        model::GuildNotificationFlag::SuppressHighlights => {
            GuildNotificationFlagPatch::NotifyHighlights(if on { 1 } else { 2 })
        }
        model::GuildNotificationFlag::MuteScheduledEvents => {
            GuildNotificationFlagPatch::MuteScheduledEvents(on)
        }
        model::GuildNotificationFlag::MobilePush => GuildNotificationFlagPatch::MobilePush(on),
    };
    guild_settings(client, guild_id).json(&body)
}

impl Rest {
    pub async fn set_guild_mute(&self, guild_id: u64, mute: Option<i64>) -> Result<(), Error> {
        let now = Utc::now();
        self.send(|| set_guild_mute(&self.client, guild_id, mute, now))
            .await?;
        Ok(())
    }

    pub async fn set_guild_notifications(
        &self,
        guild_id: u64,
        level: model::NotificationLevel,
    ) -> Result<(), Error> {
        self.send(|| set_guild_notifications(&self.client, guild_id, level))
            .await?;
        Ok(())
    }

    pub async fn set_guild_notification_flag(
        &self,
        guild_id: u64,
        flag: model::GuildNotificationFlag,
        on: bool,
    ) -> Result<(), Error> {
        self.send(|| set_guild_notification_flag(&self.client, guild_id, flag, on))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn assert_request(builder: RequestBuilder, method: reqwest::Method, url: &str, body: &[u8]) {
        let request = builder.build().unwrap();
        assert_eq!(request.method(), method);
        assert_eq!(request.url().as_str(), url);
        assert_eq!(request.headers()[REFERER], "https://discord.com/channels/7");
        assert_eq!(
            request.headers()[reqwest::header::CONTENT_TYPE],
            "application/json"
        );
        assert_eq!(request.body().and_then(reqwest::Body::as_bytes), Some(body));
    }

    #[test]
    fn guild_mute_windows_use_the_fixed_time() {
        let client = Client::new();
        let now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();

        assert_request(
            set_guild_mute(&client, 7, Some(-1), now),
            reqwest::Method::PATCH,
            "https://discord.com/api/v9/users/@me/guilds/7/settings",
            br#"{"muted":true,"mute_config":{"selected_time_window":-1,"end_time":null}}"#,
        );
        assert_request(
            set_guild_mute(&client, 7, Some(3600), now),
            reqwest::Method::PATCH,
            "https://discord.com/api/v9/users/@me/guilds/7/settings",
            br#"{"muted":true,"mute_config":{"selected_time_window":3600,"end_time":"2024-01-01T01:00:00.000Z"}}"#,
        );
        assert_request(
            set_guild_mute(&client, 7, None, now),
            reqwest::Method::PATCH,
            "https://discord.com/api/v9/users/@me/guilds/7/settings",
            br#"{"muted":false}"#,
        );
    }

    #[test]
    fn guild_notification_levels_are_explicit() {
        let client = Client::new();
        for (level, body) in [
            (
                model::NotificationLevel::All,
                br#"{"message_notifications":0}"#.as_slice(),
            ),
            (
                model::NotificationLevel::Mentions,
                br#"{"message_notifications":1}"#.as_slice(),
            ),
            (
                model::NotificationLevel::Nothing,
                br#"{"message_notifications":2}"#.as_slice(),
            ),
        ] {
            assert_request(
                set_guild_notifications(&client, 7, level),
                reqwest::Method::PATCH,
                "https://discord.com/api/v9/users/@me/guilds/7/settings",
                body,
            );
        }
    }

    #[test]
    #[should_panic(expected = "Guild notification levels must be explicit")]
    fn guild_notification_builder_rejects_inherit() {
        let _ = set_guild_notifications(&Client::new(), 7, model::NotificationLevel::Inherit);
    }

    fn assert_flag_requests(flag: model::GuildNotificationFlag, on_body: &[u8], off_body: &[u8]) {
        let client = Client::new();
        for (on, body) in [(true, on_body), (false, off_body)] {
            assert_request(
                set_guild_notification_flag(&client, 7, flag, on),
                reqwest::Method::PATCH,
                "https://discord.com/api/v9/users/@me/guilds/7/settings",
                body,
            );
        }
    }

    #[test]
    fn suppress_everyone_patches_only_its_flag() {
        assert_flag_requests(
            model::GuildNotificationFlag::SuppressEveryone,
            br#"{"suppress_everyone":true}"#,
            br#"{"suppress_everyone":false}"#,
        );
    }

    #[test]
    fn suppress_roles_patches_only_its_flag() {
        assert_flag_requests(
            model::GuildNotificationFlag::SuppressRoles,
            br#"{"suppress_roles":true}"#,
            br#"{"suppress_roles":false}"#,
        );
    }

    #[test]
    fn suppress_highlights_patches_disabled_or_enabled() {
        assert_flag_requests(
            model::GuildNotificationFlag::SuppressHighlights,
            br#"{"notify_highlights":1}"#,
            br#"{"notify_highlights":2}"#,
        );
    }

    #[test]
    fn mute_scheduled_events_patches_only_its_flag() {
        assert_flag_requests(
            model::GuildNotificationFlag::MuteScheduledEvents,
            br#"{"mute_scheduled_events":true}"#,
            br#"{"mute_scheduled_events":false}"#,
        );
    }

    #[test]
    fn mobile_push_patches_only_its_flag() {
        assert_flag_requests(
            model::GuildNotificationFlag::MobilePush,
            br#"{"mobile_push":true}"#,
            br#"{"mobile_push":false}"#,
        );
    }
}
