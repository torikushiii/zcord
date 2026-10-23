use super::{Error, OutgoingMessage, ProfileKind, model, properties};
use reqwest::{
    Client, RequestBuilder, Response,
    header::{AUTHORIZATION, HeaderMap, HeaderValue, REFERER, USER_AGENT},
};
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use tokio::{
    sync::Mutex,
    time::{Instant, sleep_until},
};

pub(super) struct Rest {
    client: Client,
    // A single serialized gate conservatively honors route AND global limits across all clones.
    gate: Mutex<Gate>,
}

struct Gate {
    next_request: Instant,
    unauthorized: bool,
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
            gate: Mutex::new(Gate {
                next_request: Instant::now(),
                unauthorized: false,
            }),
        })
    }

    pub async fn fetch_messages(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<model::Message>, Error> {
        let referer = referer(guild_id, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/messages");
        let mut query = Vec::new();
        if let Some(before) = before {
            query.push(("before", before.to_string()));
        }
        query.push(("limit", limit.to_string()));
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
    ) -> Result<(), Error> {
        let referer = referer(guild_id, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/typing");
        {
            let Ok(mut gate) = self.gate.try_lock() else {
                return Ok(());
            };
            if gate.unauthorized {
                return Err(Error::Http(401));
            }
            if gate.next_request > Instant::now() {
                return Ok(());
            }
            gate.next_request = Instant::now() + Duration::from_secs(1);
        }
        let response = self
            .client
            .post(&url)
            .header(REFERER, &referer)
            .send()
            .await
            .map_err(|_| Error::Network)?;
        let status = response.status();
        if status.as_u16() == 401 {
            if let Ok(mut gate) = self.gate.try_lock() {
                gate.unauthorized = true;
            }
            return Err(Error::Http(401));
        }
        if !status.is_success() {
            let value = response.json::<Value>().await.unwrap_or(Value::Null);
            return Err(classify(status.as_u16(), &value));
        }
        Ok(())
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
        let mut gate = self.gate.lock().await;
        if gate.unauthorized {
            return Err(Error::Http(401));
        }
        for attempt in 0..3 {
            sleep_until(gate.next_request).await;
            gate.next_request = Instant::now() + Duration::from_secs(1);
            let response = request().send().await.map_err(|_| Error::Network)?;
            let status = response.status();
            if status.as_u16() == 401 {
                gate.unauthorized = true;
            }
            let headers = response.headers();
            let retry_header = seconds(
                headers
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok()),
            );
            if headers
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok())
                == Some("0")
            {
                let reset = seconds(
                    headers
                        .get("x-ratelimit-reset-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse().ok()),
                );
                if let Some(reset) = reset {
                    gate.next_request = gate.next_request.max(Instant::now() + reset);
                }
            }
            if status.as_u16() == 429 {
                let value = response.json::<Value>().await.unwrap_or(Value::Null);
                let body_retry = seconds(value.get("retry_after").and_then(Value::as_f64));
                let delay = retry_header
                    .into_iter()
                    .chain(body_retry)
                    .max()
                    .unwrap_or(Duration::from_secs(5))
                    .max(Duration::from_secs(1));
                gate.next_request = gate.next_request.max(Instant::now() + delay);
                if attempt == 2 {
                    return Err(Error::RateLimited { retry_after: delay });
                }
                continue;
            }
            if !status.is_success() {
                if let Some(delay) = retry_header {
                    gate.next_request = gate.next_request.max(Instant::now() + delay);
                }
                if status.as_u16() == 401 {
                    return Err(Error::Http(401));
                }
                let value = response.json::<Value>().await.unwrap_or(Value::Null);
                return Err(classify(status.as_u16(), &value));
            }
            return Ok(response);
        }
        Err(Error::Decode)
    }
}

const ACK_BULK_LIMIT: usize = 100;

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
        ("with_mutual_friends", false.to_string()),
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
            "type=modal&with_mutual_guilds=true&with_mutual_friends=false&with_mutual_friends_count=true&guild_id=1191168956170240010"
        );
        assert_eq!(
            encode(ProfileKind::Modal, None),
            "type=modal&with_mutual_guilds=true&with_mutual_friends=false&with_mutual_friends_count=true"
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
