use super::*;
use chrono::{DateTime, Utc};

const DM_REFERER: &str = "https://discord.com/channels/@me";
const RELATIONSHIPS_URL: &str = "https://discord.com/api/v9/users/@me/relationships";
const CONTEXT_MENU: &str = "eyJsb2NhdGlvbiI6IkNvbnRleHRNZW51In0=";
const ADD_FRIENDS_TO_DM: &str = "eyJsb2NhdGlvbiI6IkFkZCBGcmllbmRzIHRvIERNIn0=";

#[derive(Serialize)]
struct EmptyBody {}

#[derive(Serialize)]
struct Nickname<'a> {
    nickname: Option<&'a str>,
}

#[derive(Serialize)]
struct Recipients {
    recipients: Vec<String>,
}

#[derive(Serialize)]
struct ChannelOverrides {
    channel_overrides: HashMap<String, MuteOverride>,
}

#[derive(Serialize)]
struct MuteOverride {
    muted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    mute_config: Option<MuteConfig>,
}

#[derive(Serialize)]
struct MuteConfig {
    selected_time_window: i64,
    end_time: Option<String>,
}

fn channel_url(channel_id: u64) -> String {
    format!("https://discord.com/api/v9/channels/{channel_id}")
}

fn relationship_url(user_id: u64) -> String {
    format!("{RELATIONSHIPS_URL}/{user_id}")
}

fn close_dm(client: &Client, channel_id: u64) -> RequestBuilder {
    client
        .delete(format!("{}?silent=false", channel_url(channel_id)))
        .header(REFERER, DM_REFERER)
}

fn leave_group(client: &Client, channel_id: u64, silent: bool) -> RequestBuilder {
    client
        .delete(format!("{}?silent={silent}", channel_url(channel_id)))
        .header(REFERER, DM_REFERER)
}

fn block_user(client: &Client, user_id: u64) -> RequestBuilder {
    client
        .put(relationship_url(user_id))
        .header(REFERER, DM_REFERER)
        .header("x-context-properties", CONTEXT_MENU)
        .json(&serde_json::json!({"type": 2}))
}

fn unblock_user(client: &Client, user_id: u64) -> RequestBuilder {
    client
        .delete(relationship_url(user_id))
        .header(REFERER, DM_REFERER)
        .header("x-context-properties", CONTEXT_MENU)
}

fn ignore_user(client: &Client, user_id: u64) -> RequestBuilder {
    client
        .put(format!("{}/ignore", relationship_url(user_id)))
        .header(REFERER, DM_REFERER)
        .json(&EmptyBody {})
}

fn unignore_user(client: &Client, user_id: u64) -> RequestBuilder {
    client
        .delete(format!("{}/ignore", relationship_url(user_id)))
        .header(REFERER, DM_REFERER)
}

fn set_friend_nickname(client: &Client, user_id: u64, nickname: Option<&str>) -> RequestBuilder {
    client
        .patch(relationship_url(user_id))
        .header(REFERER, DM_REFERER)
        .json(&Nickname { nickname })
}

fn set_dm_mute(
    client: &Client,
    channel_id: u64,
    mute: Option<i64>,
    now: DateTime<Utc>,
) -> RequestBuilder {
    let (muted, mute_config) = match mute {
        Some(seconds) => (
            true,
            Some(MuteConfig {
                selected_time_window: seconds,
                end_time: (seconds != -1).then(|| {
                    (now + chrono::Duration::seconds(seconds))
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                }),
            }),
        ),
        None => (false, None),
    };
    let body = ChannelOverrides {
        channel_overrides: HashMap::from([(
            channel_id.to_string(),
            MuteOverride { muted, mute_config },
        )]),
    };
    client
        .patch("https://discord.com/api/v9/users/@me/guilds/@me/settings")
        .header(REFERER, DM_REFERER)
        .json(&body)
}

fn create_group_dm(client: &Client, recipients: &[u64]) -> RequestBuilder {
    let body = Recipients {
        recipients: recipients.iter().map(|id| id.to_string()).collect(),
    };
    client
        .post("https://discord.com/api/v9/users/@me/channels")
        .header(REFERER, DM_REFERER)
        .header("x-context-properties", ADD_FRIENDS_TO_DM)
        .json(&body)
}

fn add_group_recipient(client: &Client, channel_id: u64, user_id: u64) -> RequestBuilder {
    client
        .put(format!("{}/recipients/{user_id}", channel_url(channel_id)))
        .header(REFERER, DM_REFERER)
        .json(&EmptyBody {})
}

fn remove_group_recipient(client: &Client, channel_id: u64, user_id: u64) -> RequestBuilder {
    client
        .delete(format!("{}/recipients/{user_id}", channel_url(channel_id)))
        .header(REFERER, DM_REFERER)
}

impl Rest {
    pub async fn close_dm(&self, channel_id: u64) -> Result<(), Error> {
        self.send(|| close_dm(&self.client, channel_id)).await?;
        Ok(())
    }

    pub async fn leave_group(&self, channel_id: u64, silent: bool) -> Result<(), Error> {
        self.send(|| leave_group(&self.client, channel_id, silent))
            .await?;
        Ok(())
    }

    pub async fn block_user(&self, user_id: u64) -> Result<(), Error> {
        self.send(|| block_user(&self.client, user_id)).await?;
        Ok(())
    }

    pub async fn unblock_user(&self, user_id: u64) -> Result<(), Error> {
        self.send(|| unblock_user(&self.client, user_id)).await?;
        Ok(())
    }

    pub async fn ignore_user(&self, user_id: u64) -> Result<(), Error> {
        self.send(|| ignore_user(&self.client, user_id)).await?;
        Ok(())
    }

    pub async fn unignore_user(&self, user_id: u64) -> Result<(), Error> {
        self.send(|| unignore_user(&self.client, user_id)).await?;
        Ok(())
    }

    pub async fn set_friend_nickname(
        &self,
        user_id: u64,
        nickname: Option<&str>,
    ) -> Result<(), Error> {
        self.send(|| set_friend_nickname(&self.client, user_id, nickname))
            .await?;
        Ok(())
    }

    pub async fn set_dm_mute(&self, channel_id: u64, mute: Option<i64>) -> Result<(), Error> {
        let now = Utc::now();
        self.send(|| set_dm_mute(&self.client, channel_id, mute, now))
            .await?;
        Ok(())
    }

    pub async fn create_group_dm(&self, recipients: &[u64]) -> Result<model::Channel, Error> {
        let response = self
            .send(|| create_group_dm(&self.client, recipients))
            .await?;
        response
            .json::<model::Channel>()
            .await
            .map_err(|_| Error::Decode)
    }

    pub async fn add_group_recipient(&self, channel_id: u64, user_id: u64) -> Result<(), Error> {
        self.send(|| add_group_recipient(&self.client, channel_id, user_id))
            .await?;
        Ok(())
    }

    pub async fn remove_group_recipient(&self, channel_id: u64, user_id: u64) -> Result<(), Error> {
        self.send(|| remove_group_recipient(&self.client, channel_id, user_id))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use reqwest::Client;

    fn assert_request(
        builder: RequestBuilder,
        method: reqwest::Method,
        url: &str,
        body: Option<&[u8]>,
        context: Option<&str>,
    ) {
        let request = builder.build().unwrap();
        assert_eq!(request.method(), method);
        assert_eq!(request.url().as_str(), url);
        assert_eq!(request.body().and_then(reqwest::Body::as_bytes), body);
        assert_eq!(request.headers()[REFERER], DM_REFERER);
        assert_eq!(
            request
                .headers()
                .get("x-context-properties")
                .map(|value| value.to_str().unwrap()),
            context
        );
    }

    #[test]
    fn dms_build_the_expected_requests() {
        let client = Client::new();

        assert_request(
            close_dm(&client, 7),
            reqwest::Method::DELETE,
            "https://discord.com/api/v9/channels/7?silent=false",
            None,
            None,
        );
        assert_request(
            leave_group(&client, 7, true),
            reqwest::Method::DELETE,
            "https://discord.com/api/v9/channels/7?silent=true",
            None,
            None,
        );
        assert_request(
            leave_group(&client, 7, false),
            reqwest::Method::DELETE,
            "https://discord.com/api/v9/channels/7?silent=false",
            None,
            None,
        );
        assert_request(
            block_user(&client, 42),
            reqwest::Method::PUT,
            "https://discord.com/api/v9/users/@me/relationships/42",
            Some(br#"{"type":2}"#),
            Some(CONTEXT_MENU),
        );
        assert_request(
            unblock_user(&client, 42),
            reqwest::Method::DELETE,
            "https://discord.com/api/v9/users/@me/relationships/42",
            None,
            Some(CONTEXT_MENU),
        );
        assert_request(
            ignore_user(&client, 42),
            reqwest::Method::PUT,
            "https://discord.com/api/v9/users/@me/relationships/42/ignore",
            Some(b"{}"),
            None,
        );
        assert_request(
            unignore_user(&client, 42),
            reqwest::Method::DELETE,
            "https://discord.com/api/v9/users/@me/relationships/42/ignore",
            None,
            None,
        );
        assert_request(
            set_friend_nickname(&client, 42, Some("nick")),
            reqwest::Method::PATCH,
            "https://discord.com/api/v9/users/@me/relationships/42",
            Some(br#"{"nickname":"nick"}"#),
            None,
        );
        assert_request(
            set_friend_nickname(&client, 42, None),
            reqwest::Method::PATCH,
            "https://discord.com/api/v9/users/@me/relationships/42",
            Some(br#"{"nickname":null}"#),
            None,
        );
        assert_request(
            create_group_dm(&client, &[1, 2]),
            reqwest::Method::POST,
            "https://discord.com/api/v9/users/@me/channels",
            Some(br#"{"recipients":["1","2"]}"#),
            Some(ADD_FRIENDS_TO_DM),
        );
        assert_request(
            add_group_recipient(&client, 7, 42),
            reqwest::Method::PUT,
            "https://discord.com/api/v9/channels/7/recipients/42",
            Some(b"{}"),
            None,
        );
        assert_request(
            remove_group_recipient(&client, 7, 42),
            reqwest::Method::DELETE,
            "https://discord.com/api/v9/channels/7/recipients/42",
            None,
            None,
        );
    }

    #[test]
    fn dm_mute_windows_use_the_fixed_time() {
        let client = Client::new();
        let now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();

        assert_request(
            set_dm_mute(&client, 7, Some(-1), now),
            reqwest::Method::PATCH,
            "https://discord.com/api/v9/users/@me/guilds/@me/settings",
            Some(
                br#"{"channel_overrides":{"7":{"muted":true,"mute_config":{"selected_time_window":-1,"end_time":null}}}}"#,
            ),
            None,
        );
        assert_request(
            set_dm_mute(&client, 7, Some(3600), now),
            reqwest::Method::PATCH,
            "https://discord.com/api/v9/users/@me/guilds/@me/settings",
            Some(
                br#"{"channel_overrides":{"7":{"muted":true,"mute_config":{"selected_time_window":3600,"end_time":"2024-01-01T01:00:00.000Z"}}}}"#,
            ),
            None,
        );
        assert_request(
            set_dm_mute(&client, 7, None, now),
            reqwest::Method::PATCH,
            "https://discord.com/api/v9/users/@me/guilds/@me/settings",
            Some(br#"{"channel_overrides":{"7":{"muted":false}}}"#),
            None,
        );
    }
}
