use super::{Error, OutgoingMessage, ProfileKind, model, properties};
use reqwest::{
    Client, Request, RequestBuilder, Response,
    header::{AUTHORIZATION, HeaderMap, HeaderValue, REFERER, USER_AGENT},
};
use serde::Serialize;
use serde_json::Value;
use std::{collections::HashMap, time::Duration};
mod rate_limits;
use rate_limits::{Limiter, Permit, Route};

pub(super) struct Rest {
    client: Client,
    limits: Limiter,
}

impl Rest {
    pub fn new(token: &str) -> Result<Self, Error> {
        let mut headers = HeaderMap::new();
        let mut authorization =
            HeaderValue::from_str(token).map_err(|_| Error::InvalidCredential)?;
        authorization.set_sensitive(true);
        headers.insert(AUTHORIZATION, authorization);
        headers.insert(USER_AGENT, HeaderValue::from_static(properties::USER_AGENT));
        for (name, value) in [
            ("accept", "*/*"),
            ("accept-language", properties::ACCEPT_LANGUAGE),
            ("priority", "u=1, i"),
            ("sec-ch-ua", properties::SEC_CH_UA),
            ("sec-ch-ua-mobile", "?0"),
            ("sec-ch-ua-platform", "\"Windows\""),
            ("sec-fetch-dest", "empty"),
            ("sec-fetch-mode", "cors"),
            ("sec-fetch-site", "same-origin"),
            ("x-debug-options", "bugReporterEnabled"),
        ] {
            headers.insert(name, HeaderValue::from_static(value));
        }
        headers.insert(
            "x-super-properties",
            HeaderValue::from_str(&properties::super_properties()).map_err(|_| Error::Decode)?,
        );
        headers.insert(
            "x-discord-locale",
            HeaderValue::from_static(properties::LOCALE),
        );
        headers.insert(
            "x-discord-timezone",
            HeaderValue::from_str(&properties::timezone()).map_err(|_| Error::Decode)?,
        );
        let client = Client::builder()
            .default_headers(headers)
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| Error::Network)?;
        Ok(Self {
            client,
            limits: Limiter::default(),
        })
    }

    pub async fn fetch_messages(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<model::Message>, Error> {
        let anchor = before.map_or(Anchor::Latest, Anchor::Before);
        self.fetch_messages_anchor(guild_id, channel_id, anchor, limit)
            .await
    }

    pub async fn fetch_forum_posts(
        &self,
        guild_id: Option<u64>,
        forum_id: u64,
        offset: usize,
    ) -> Result<model::ForumSearch, Error> {
        let referer = referer(guild_id, forum_id);
        let url = format!("https://discord.com/api/v9/channels/{forum_id}/threads/search");
        let query = forum_search_query(offset);
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .query(&query)
                    .header(REFERER, &referer)
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    async fn fetch_messages_anchor(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        anchor: Anchor,
        limit: usize,
    ) -> Result<Vec<model::Message>, Error> {
        let referer = referer(guild_id, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/messages");
        let query = history_query(anchor, limit);
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .query(&query)
                    .header(REFERER, &referer)
            })
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        if !value.is_array() {
            return Err(Error::Decode);
        }
        let mut messages: Vec<model::Message> = model::parse_entries(value);
        messages.retain(|m| m.channel_id == channel_id);
        messages.sort_by_key(|m| m.id);
        messages.dedup_by_key(|m| m.id);
        Ok(messages)
    }

    async fn add_reaction(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        emoji: &str,
    ) -> Result<(), Error> {
        let referer = referer(guild_id, channel_id);
        let url = reaction_url(channel_id, message_id, emoji);
        self.send(|| {
            self.client
                .put(&url)
                .header(REFERER, &referer)
                .query(&[("location", "Message"), ("type", "0")])
        })
        .await?;
        Ok(())
    }

    async fn remove_reaction(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        emoji: &str,
    ) -> Result<(), Error> {
        let referer = referer(guild_id, channel_id);
        let url = reaction_url(channel_id, message_id, emoji);
        self.send(|| {
            self.client
                .delete(&url)
                .header(REFERER, &referer)
                .query(&[("location", "Message"), ("type", "0")])
        })
        .await?;
        Ok(())
    }

    pub async fn ack_bulk(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        acks: &[(u64, u64)],
    ) -> Result<(), Error> {
        let referer = referer(guild_id, channel_id);
        for chunk in acks.chunks(ACK_BULK_LIMIT) {
            let read_states: Vec<_> = chunk
                .iter()
                .map(|(channel_id, message_id)| ReadStateAck {
                    channel_id: channel_id.to_string(),
                    message_id: message_id.to_string(),
                    read_state_type: 0,
                })
                .collect();
            let body = AckBulk { read_states };
            self.send(|| {
                self.client
                    .post("https://discord.com/api/v9/read-states/ack-bulk")
                    .header(REFERER, &referer)
                    .json(&body)
            })
            .await?;
        }
        Ok(())
    }

    pub async fn send_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message: &OutgoingMessage,
    ) -> Result<model::Message, Error> {
        let referer = referer(guild_id, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/messages");
        let body = message_body(guild_id, channel_id, message);
        let response = self
            .send(|| self.client.post(&url).header(REFERER, &referer).json(&body))
            .await?;
        response
            .json::<model::Message>()
            .await
            .map_err(|_| Error::Decode)
    }

    pub async fn edit_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        content: &str,
    ) -> Result<model::MessageUpdate, Error> {
        let referer = referer(guild_id, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/messages/{message_id}");
        let body = EditMessage { content };
        let response = self
            .send(|| {
                self.client
                    .patch(&url)
                    .header(REFERER, &referer)
                    .json(&body)
            })
            .await?;
        response
            .json::<model::MessageUpdate>()
            .await
            .map_err(|_| Error::Decode)
    }

    pub async fn delete_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
    ) -> Result<(), Error> {
        let referer = referer(guild_id, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/messages/{message_id}");
        self.send(|| self.client.delete(&url).header(REFERER, &referer))
            .await?;
        Ok(())
    }

    pub async fn trigger_typing(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
    ) -> Result<bool, Error> {
        let referer = referer(guild_id, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/typing");
        let request = self
            .client
            .post(&url)
            .header(REFERER, &referer)
            .build()
            .map_err(|_| Error::Network)?;
        let Some(permit) = self.limits.try_acquire(Route::of(&request))? else {
            return Ok(false);
        };
        self.execute(request, permit).await?;
        Ok(true)
    }

    pub async fn ack_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        days_since_discord_epoch: u32,
    ) -> Result<(), Error> {
        let referer = referer(guild_id, channel_id);
        let url =
            format!("https://discord.com/api/v9/channels/{channel_id}/messages/{message_id}/ack");
        let body = AckMessage {
            token: None,
            last_viewed: days_since_discord_epoch,
        };
        self.send(|| self.client.post(&url).header(REFERER, &referer).json(&body))
            .await?;
        Ok(())
    }

    pub async fn fetch_profile(
        &self,
        user_id: u64,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
        kind: ProfileKind,
    ) -> Result<model::Profile, Error> {
        let referer = channel_id.map_or_else(
            || "https://discord.com/channels/@me".to_owned(),
            |channel_id| referer(guild_id, channel_id),
        );
        let url = format!("https://discord.com/api/v9/users/{user_id}/profile");
        let query = profile_query(kind, guild_id);
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .query(&query)
                    .header(REFERER, &referer)
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn fetch_user_badges(
        &self,
        user_id: u64,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
    ) -> Result<model::UserBadges, Error> {
        let referer = channel_id.map_or_else(
            || "https://discord.com/channels/@me".to_owned(),
            |channel_id| referer(guild_id, channel_id),
        );
        let url = format!("https://discord.com/api/v9/users/{user_id}/badges");
        let response = self
            .send(|| self.client.get(&url).header(REFERER, &referer))
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn fetch_outbox(&self, user_id: u64) -> Result<model::Outbox, Error> {
        let url = format!("https://discord.com/api/v9/content-inventory/users/{user_id}/outbox");
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn fetch_application_identities(
        &self,
        user_id: u64,
    ) -> Result<model::ApplicationIdentities, Error> {
        let url = format!("https://discord.com/api/v9/users/{user_id}/application-identities");
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .query(&[("with_profiles", "true")])
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn fetch_wishlist(&self, wishlist_id: u64) -> Result<model::Wishlist, Error> {
        let url = format!("https://discord.com/api/v9/wishlists/{wishlist_id}");
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .query(&[("source", "user_profile")])
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    /// `Ok(None)` means the user has no note; Discord answers 404 with code 10013.
    pub async fn fetch_note(&self, user_id: u64) -> Result<Option<String>, Error> {
        let url = format!("https://discord.com/api/v9/users/@me/notes/{user_id}");
        let response = match self
            .send(|| {
                self.client
                    .get(&url)
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await
        {
            Ok(response) => response,
            Err(Error::Http(404)) | Err(Error::Rejected { status: 404, .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        Ok(value
            .get("note")
            .and_then(Value::as_str)
            .filter(|note| !note.is_empty())
            .map(str::to_owned))
    }

    /// `Ok(None)` means Discord rejected the code (404, invite error 10006).
    pub async fn fetch_invite(&self, code: &str) -> Result<Option<model::Invite>, Error> {
        if code.len() < 2
            || code.len() > 32
            || !code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Ok(None);
        }
        let url = format!("https://discord.com/api/v9/invites/{code}");
        let response = match self
            .send(|| {
                self.client
                    .get(&url)
                    .query(&[("with_counts", "true"), ("with_expiration", "true")])
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await
        {
            Ok(response) => response,
            Err(Error::Http(404)) | Err(Error::Rejected { status: 404, .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        response.json().await.map_err(|_| Error::Decode).map(Some)
    }

    pub async fn fetch_games(&self, ids: &[u64]) -> Result<Vec<model::Game>, Error> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        // Discord answers at most 20 ids per request (its own client batches by 20); the rest
        // would otherwise come back absent and be cached as missing.
        let mut games = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(LOOKUP_BATCH) {
            let query: Vec<_> = chunk.iter().map(|id| ("game_ids", id.to_string())).collect();
            let response = self
                .send(|| {
                    self.client
                        .get("https://discord.com/api/v9/games")
                        .query(&query)
                        .header(REFERER, "https://discord.com/channels/@me")
                })
                .await?;
            let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
            games.extend(model::parse_entries(value));
        }
        Ok(games)
    }

    pub async fn fetch_public_applications(
        &self,
        ids: &[u64],
    ) -> Result<Vec<model::PublicApplication>, Error> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut applications = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(LOOKUP_BATCH) {
            let query: Vec<_> = chunk
                .iter()
                .map(|id| ("application_ids", id.to_string()))
                .collect();
            let response = self
                .send(|| {
                    self.client
                        .get("https://discord.com/api/v9/applications/public")
                        .query(&query)
                        .header(REFERER, "https://discord.com/channels/@me")
                })
                .await?;
            let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
            applications.extend(model::parse_entries(value));
        }
        Ok(applications)
    }

    pub async fn fetch_featured_widget_configs(
        &self,
    ) -> Result<HashMap<u64, model::WidgetConfig>, Error> {
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/widget-configs/featured")
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        Ok(model::parse_featured_widget_configs(value))
    }

    pub async fn fetch_application_widget_configs(
        &self,
        application_id: u64,
    ) -> Result<Option<model::WidgetConfig>, Error> {
        let url =
            format!("https://discord.com/api/v9/applications/{application_id}/widget-configs");
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        Ok(model::parse_application_widget_configs(
            value,
            application_id,
        ))
    }

    pub async fn fetch_collectible_product(
        &self,
        sku_id: u64,
    ) -> Result<model::CollectibleProduct, Error> {
        let url = format!("https://discord.com/api/v9/collectibles-products/{sku_id}");
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .query(&[("locale", properties::LOCALE)])
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    async fn send(&self, request: impl Fn() -> RequestBuilder) -> Result<Response, Error> {
        for attempt in 0..3 {
            let request = request().build().map_err(|_| Error::Network)?;
            let permit = self.limits.acquire(Route::of(&request)).await?;
            match self.execute(request, permit).await {
                Err(Error::RateLimited { .. }) if attempt < 2 => continue,
                result => return result,
            }
        }
        unreachable!()
    }

    async fn execute(&self, request: Request, permit: Permit<'_>) -> Result<Response, Error> {
        let response = self
            .client
            .execute(request)
            .await
            .map_err(|_| Error::Network)?;
        let status = response.status();
        permit.observe(status.as_u16(), response.headers());
        if status.as_u16() == 401 {
            return Err(Error::Http(401));
        }
        if status.as_u16() == 429 {
            let headers = response.headers().clone();
            let body = response.json::<Value>().await.unwrap_or(Value::Null);
            return Err(Error::RateLimited {
                retry_after: permit.retry(&headers, &body),
            });
        }
        if !status.is_success() {
            let value = response.json::<Value>().await.unwrap_or(Value::Null);
            return Err(classify(status.as_u16(), &value));
        }
        Ok(response)
    }
}

const ACK_BULK_LIMIT: usize = 100;
const LOOKUP_BATCH: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Anchor {
    Before(u64),
    After(u64),
    Around(u64),
    Latest,
}

fn history_query(anchor: Anchor, limit: usize) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    match anchor {
        Anchor::Before(id) => query.push(("before", id.to_string())),
        Anchor::After(id) => query.push(("after", id.to_string())),
        Anchor::Around(id) => query.push(("around", id.to_string())),
        Anchor::Latest => {}
    }
    query.push(("limit", limit.to_string()));
    query
}

// `archived` is omitted on purpose: `true` restricts the search to archived posts only, and
// leaving it out searches active and archived posts together.
fn forum_search_query(offset: usize) -> Vec<(&'static str, String)> {
    vec![
        ("sort_by", "last_message_time".to_owned()),
        ("sort_order", "desc".to_owned()),
        ("limit", "25".to_owned()),
        ("tag_setting", "match_some".to_owned()),
        ("offset", offset.to_string()),
    ]
}

impl super::Session {
    pub async fn fetch_forum_posts(
        &self,
        guild_id: Option<u64>,
        forum_id: u64,
        offset: usize,
    ) -> Result<model::ForumSearch, Error> {
        self.rest(async |rest| rest.fetch_forum_posts(guild_id, forum_id, offset).await)
            .await
    }

    pub async fn fetch_messages_after(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        after: u64,
        limit: usize,
    ) -> Result<Vec<model::Message>, Error> {
        self.rest(async |rest| {
            rest.fetch_messages_anchor(guild_id, channel_id, Anchor::After(after), limit)
                .await
        })
        .await
    }

    pub async fn fetch_messages_around(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        around: u64,
        limit: usize,
    ) -> Result<Vec<model::Message>, Error> {
        self.rest(async |rest| {
            rest.fetch_messages_anchor(guild_id, channel_id, Anchor::Around(around), limit)
                .await
        })
        .await
    }

    pub async fn add_reaction(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        emoji: &str,
    ) -> Result<(), Error> {
        self.rest(async |rest| {
            rest.add_reaction(guild_id, channel_id, message_id, emoji)
                .await
        })
        .await
    }

    pub async fn remove_reaction(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        emoji: &str,
    ) -> Result<(), Error> {
        self.rest(async |rest| {
            rest.remove_reaction(guild_id, channel_id, message_id, emoji)
                .await
        })
        .await
    }
}

#[derive(Serialize)]
struct CreateMessage<'a> {
    mobile_network_type: &'static str,
    content: &'a str,
    nonce: String,
    tts: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_reference: Option<MessageReferenceBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_mentions: Option<AllowedMentions>,
    flags: u64,
}

#[derive(Serialize)]
struct MessageReferenceBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    guild_id: Option<String>,
    channel_id: String,
    message_id: String,
    #[serde(rename = "type")]
    kind: u8,
}

// `parse` must stay explicit: with any allowed_mentions object present, an omitted
// `parse` suppresses content mentions, so a non-pinging reply would stop pinging @user.
#[derive(Serialize)]
struct AllowedMentions {
    parse: [&'static str; 3],
    replied_user: bool,
}

#[derive(Serialize)]
struct EditMessage<'a> {
    content: &'a str,
}

fn message_body<'a>(
    guild_id: Option<u64>,
    channel_id: u64,
    message: &'a OutgoingMessage,
) -> CreateMessage<'a> {
    CreateMessage {
        mobile_network_type: "unknown",
        content: &message.content,
        nonce: message.nonce.to_string(),
        tts: false,
        message_reference: message.reply.as_ref().map(|reply| MessageReferenceBody {
            guild_id: guild_id.map(|id| id.to_string()),
            channel_id: channel_id.to_string(),
            message_id: reply.message_id.to_string(),
            kind: 0,
        }),
        allowed_mentions: message
            .reply
            .as_ref()
            .filter(|reply| !reply.mention)
            .map(|_| AllowedMentions {
                parse: ["users", "roles", "everyone"],
                replied_user: false,
            }),
        flags: 0,
    }
}

fn classify(status: u16, value: &Value) -> Error {
    if status == 401 {
        return Error::Http(401);
    }
    if value.get("captcha_key").is_some_and(|key| !key.is_null()) {
        return Error::Captcha;
    }
    if let (Some(code), Some(message)) = (
        value.get("code").and_then(Value::as_u64),
        value.get("message").and_then(Value::as_str),
    ) {
        return Error::Rejected {
            status,
            code,
            message: message.to_owned(),
        };
    }
    Error::Http(status)
}

#[derive(Serialize)]
struct AckMessage {
    token: Option<String>,
    last_viewed: u32,
}

#[derive(Serialize)]
struct AckBulk {
    read_states: Vec<ReadStateAck>,
}

#[derive(Serialize)]
struct ReadStateAck {
    channel_id: String,
    message_id: String,
    read_state_type: u8,
}

// Same parameters, in the same order, as the official client's popout and full profile requests.
fn profile_query(kind: ProfileKind, guild_id: Option<u64>) -> Vec<(&'static str, String)> {
    let modal = kind == ProfileKind::Modal;
    let mut query = vec![
        ("type", if modal { "modal" } else { "popout" }.to_owned()),
        ("with_mutual_guilds", modal.to_string()),
        ("with_mutual_friends", modal.to_string()),
        ("with_mutual_friends_count", modal.to_string()),
    ];
    if let Some(guild_id) = guild_id {
        query.push(("guild_id", guild_id.to_string()));
    }
    query
}

fn referer(guild_id: Option<u64>, channel_id: u64) -> String {
    match guild_id {
        Some(guild_id) => format!("https://discord.com/channels/{guild_id}/{channel_id}"),
        None => format!("https://discord.com/channels/@me/{channel_id}"),
    }
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for &byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn reaction_url(channel_id: u64, message_id: u64, emoji: &str) -> String {
    format!(
        "https://discord.com/api/v9/channels/{channel_id}/messages/{message_id}/reactions/{}/@me",
        percent_encode(emoji)
    )
}

fn seconds(value: Option<f64>) -> Option<Duration> {
    value
        .filter(|v| v.is_finite() && *v >= 0.0)
        .and_then(|v| Duration::try_from_secs_f64(v).ok())
        .filter(|v| *v <= Duration::from_secs(365 * 24 * 60 * 60))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::OutgoingReply;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        time::{Instant, timeout},
    };

    fn local_rest() -> Rest {
        Rest {
            client: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            limits: Limiter::default(),
        }
    }

    async fn accept_request(listener: &TcpListener) -> TcpStream {
        timeout(Duration::from_secs(2), async {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut bytes = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = stream.read(&mut bytes).await.unwrap();
                assert!(count > 0 && request.len() < 8192);
                request.extend_from_slice(&bytes[..count]);
            }
            stream
        })
        .await
        .unwrap()
    }

    async fn respond(mut stream: TcpStream, status: &str, headers: &str, body: &str) {
        stream.write_all(format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
            body.len(),
        ).as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn independent_requests_start_before_either_response_arrives() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/api/v9", listener.local_addr().unwrap());
        let profile = format!("{base}/users/1/profile");
        let outbox = format!("{base}/content-inventory/users/1/outbox");
        let rest = local_rest();
        let server = async {
            let first = accept_request(&listener).await;
            let second = accept_request(&listener).await;
            respond(first, "200 OK", "", "{}").await;
            respond(second, "200 OK", "", "{}").await;
        };
        let clients = async {
            let (profile, outbox) = tokio::join!(
                rest.send(|| rest.client.get(&profile)),
                rest.send(|| rest.client.get(&outbox)),
            );
            assert!(profile.unwrap().status().is_success());
            assert!(outbox.unwrap().status().is_success());
        };
        tokio::join!(server, clients);
    }

    #[tokio::test]
    async fn successful_responses_with_no_remaining_requests_wait_for_reset() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/api/v9/users/1/profile",
            listener.local_addr().unwrap()
        );
        let rest = local_rest();
        let server = async {
            let first = accept_request(&listener).await;
            let started = Instant::now();
            respond(first, "200 OK", "X-RateLimit-Bucket: profile\r\nX-RateLimit-Remaining: 0\r\nX-RateLimit-Reset-After: 0.05\r\n", "{}").await;
            let second = accept_request(&listener).await;
            assert!(started.elapsed() >= Duration::from_millis(50));
            respond(second, "200 OK", "", "{}").await;
        };
        let client = async {
            rest.send(|| rest.client.get(&url)).await.unwrap();
            rest.send(|| rest.client.get(&url)).await.unwrap();
        };
        tokio::join!(server, client);
    }

    #[tokio::test]
    async fn rate_limited_requests_retry_after_the_body_delay() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/api/v9/users/1/profile",
            listener.local_addr().unwrap()
        );
        let rest = local_rest();
        let server = async {
            let first = accept_request(&listener).await;
            let started = Instant::now();
            respond(
                first,
                "429 Too Many Requests",
                "X-RateLimit-Bucket: profile\r\nRetry-After: 0.01\r\n",
                r#"{"retry_after":0.05,"global":true}"#,
            )
            .await;
            let second = accept_request(&listener).await;
            assert!(started.elapsed() >= Duration::from_millis(50));
            respond(second, "200 OK", "", "{}").await;
        };
        let ((), result) = tokio::join!(server, rest.send(|| rest.client.get(&url)));
        assert!(result.unwrap().status().is_success());
    }

    #[tokio::test]
    async fn repeated_rate_limits_have_a_bounded_retry_count() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/api/v9/users/1/profile",
            listener.local_addr().unwrap()
        );
        let rest = local_rest();
        let server = async {
            for _ in 0..3 {
                let stream = accept_request(&listener).await;
                respond(stream, "429 Too Many Requests", "", r#"{"retry_after":0}"#).await;
            }
        };
        let ((), result) = tokio::join!(server, rest.send(|| rest.client.get(&url)));
        assert!(matches!(result, Err(Error::RateLimited { .. })));
    }

    #[tokio::test]
    async fn unauthorized_response_stops_subsequent_dispatches() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/api/v9/users/1/profile",
            listener.local_addr().unwrap()
        );
        let rest = local_rest();
        let server = async {
            let stream = accept_request(&listener).await;
            respond(stream, "401 Unauthorized", "", "{}").await;
        };
        let ((), result) = tokio::join!(server, rest.send(|| rest.client.get(&url)));
        assert!(matches!(result, Err(Error::Http(401))));
        let result = rest
            .send(|| rest.client.get(format!("{url}/different-route")))
            .await;
        assert!(matches!(result, Err(Error::Http(401))));
        assert!(
            timeout(Duration::from_millis(10), listener.accept())
                .await
                .is_err()
        );
    }

    #[test]
    fn emoji_api_reaction_path_encodes_custom_and_unicode() {
        assert_eq!(
            reaction_url(20, 30, "wave:123"),
            "https://discord.com/api/v9/channels/20/messages/30/reactions/wave%3A123/@me"
        );
        assert_eq!(
            reaction_url(20, 30, "\u{1F44D}"),
            "https://discord.com/api/v9/channels/20/messages/30/reactions/%F0%9F%91%8D/@me"
        );
    }

    #[test]
    fn open_at_query_builds_history_anchors() {
        let encode = |anchor| {
            history_query(anchor, 50)
                .into_iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("&")
        };
        assert_eq!(encode(Anchor::Latest), "limit=50");
        assert_eq!(encode(Anchor::Before(61)), "before=61&limit=50");
        assert_eq!(encode(Anchor::After(61)), "after=61&limit=50");
        assert_eq!(encode(Anchor::Around(61)), "around=61&limit=50");
    }

    #[test]
    fn forum_search_query_matches_the_user_client() {
        let query = forum_search_query(25)
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("&");
        assert_eq!(
            query,
            "sort_by=last_message_time&sort_order=desc&limit=25&tag_setting=match_some&offset=25"
        );
    }

    #[test]
    fn profile_queries_match_the_official_client() {
        let encode = |kind, guild_id| {
            profile_query(kind, guild_id)
                .into_iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("&")
        };
        assert_eq!(
            encode(ProfileKind::Popout, Some(1191168956170240010)),
            "type=popout&with_mutual_guilds=false&with_mutual_friends=false&with_mutual_friends_count=false&guild_id=1191168956170240010"
        );
        assert_eq!(
            encode(ProfileKind::Modal, Some(1191168956170240010)),
            "type=modal&with_mutual_guilds=true&with_mutual_friends=true&with_mutual_friends_count=true&guild_id=1191168956170240010"
        );
        assert_eq!(
            encode(ProfileKind::Modal, None),
            "type=modal&with_mutual_guilds=true&with_mutual_friends=true&with_mutual_friends_count=true"
        );
    }

    fn outgoing(reply: Option<OutgoingReply>) -> OutgoingMessage {
        OutgoingMessage {
            content: "hi".into(),
            nonce: 123,
            reply,
        }
    }

    fn body_json(reply: Option<OutgoingReply>, guild_id: Option<u64>) -> String {
        serde_json::to_string(&message_body(guild_id, 20, &outgoing(reply))).unwrap()
    }

    #[test]
    fn send_payload_field_order_and_reply_mentions() {
        assert_eq!(
            body_json(None, Some(10)),
            r#"{"mobile_network_type":"unknown","content":"hi","nonce":"123","tts":false,"flags":0}"#
        );
        let ping = Some(OutgoingReply {
            message_id: 30,
            mention: true,
        });
        assert_eq!(
            body_json(ping, Some(10)),
            r#"{"mobile_network_type":"unknown","content":"hi","nonce":"123","tts":false,"message_reference":{"guild_id":"10","channel_id":"20","message_id":"30","type":0},"flags":0}"#
        );
        let no_ping = Some(OutgoingReply {
            message_id: 30,
            mention: false,
        });
        assert_eq!(
            body_json(no_ping, Some(10)),
            r#"{"mobile_network_type":"unknown","content":"hi","nonce":"123","tts":false,"message_reference":{"guild_id":"10","channel_id":"20","message_id":"30","type":0},"allowed_mentions":{"parse":["users","roles","everyone"],"replied_user":false},"flags":0}"#
        );
        assert_eq!(
            body_json(
                Some(OutgoingReply {
                    message_id: 30,
                    mention: true
                }),
                None
            ),
            r#"{"mobile_network_type":"unknown","content":"hi","nonce":"123","tts":false,"message_reference":{"channel_id":"20","message_id":"30","type":0},"flags":0}"#
        );
    }

    #[test]
    fn error_body_classification() {
        assert!(matches!(
            classify(
                400,
                &serde_json::json!({"captcha_key":["x"],"captcha_sitekey":"k"})
            ),
            Error::Captcha
        ));
        assert!(matches!(
            classify(
                400,
                &serde_json::json!({"code":50035,"message":"Invalid Form Body"})
            ),
            Error::Rejected {
                status: 400,
                code: 50035,
                ..
            }
        ));
        assert!(matches!(classify(400, &Value::Null), Error::Http(400)));
        assert!(matches!(
            classify(
                401,
                &serde_json::json!({"message":"401: Unauthorized","code":0})
            ),
            Error::Http(401)
        ));
        assert!(matches!(
            classify(
                400,
                &serde_json::json!({"captcha_key":null,"code":50035,"message":"x"})
            ),
            Error::Rejected { .. }
        ));
    }

    #[test]
    fn fractional_retry_after_is_seconds() {
        assert_eq!(seconds(Some(1.25)), Some(Duration::from_millis(1250)));
        assert_eq!(seconds(Some(f64::NAN)), None);
        assert_eq!(seconds(Some(-1.0)), None);
    }
}
