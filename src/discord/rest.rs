use super::{Error, OutgoingMessage, ProfileKind, model, properties};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{
    Client, Request, RequestBuilder, Response,
    header::{AUTHORIZATION, HeaderMap, HeaderValue, REFERER, USER_AGENT},
};
use serde::Serialize;
use serde_json::Value;
use std::{collections::HashMap, time::Duration};
mod gifs;
mod messages;
mod profiles;
mod rate_limits;
mod requests;
mod settings;
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

const MESSAGE_REQUEST_PREVIEW_LIMIT: usize = 25;

const LOOKUP_BATCH: usize = 20;

const FRECENCY_SETTINGS: &str = "https://discord.com/api/v9/users/@me/settings-proto/2";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Anchor {
    Before(u64),
    After(u64),
    Around(u64),
    Latest,
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

fn seconds(value: Option<f64>) -> Option<Duration> {
    value
        .filter(|v| v.is_finite() && *v >= 0.0)
        .and_then(|v| Duration::try_from_secs_f64(v).ok())
        .filter(|v| *v <= Duration::from_secs(365 * 24 * 60 * 60))
}

#[cfg(test)]
mod tests {
    use super::messages::{forum_search_query, history_query, message_body, reaction_url};
    use super::profiles::profile_query;
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
