use super::{
    Error, OutgoingMessage, ProfileKind, UploadFile, UploadProgress, UploadTarget, model,
    properties,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{
    Client, Request, RequestBuilder, Response,
    header::{AUTHORIZATION, HeaderMap, HeaderValue, REFERER, USER_AGENT},
};
use serde::Serialize;
use serde_json::Value;
use std::{collections::HashMap, time::Duration};
mod expressions;
mod gifs;
mod messages;
mod pins;
mod profiles;
mod rate_limits;
mod requests;
mod settings;
mod streams;
mod threads;
pub(crate) use messages::{put_upload, upload_client};
use rate_limits::{Limiter, Permit, Route};

pub(super) struct Rest {
    client: Client,
    pub(super) upload_client: Client,
    limits: Limiter,
}

impl Rest {
    pub fn new(token: &str) -> Result<Self, Error> {
        let mut authorization =
            HeaderValue::from_str(token).map_err(|_| Error::InvalidCredential)?;
        authorization.set_sensitive(true);
        let mut headers = client_headers()?;
        headers.insert(AUTHORIZATION, authorization);
        let client = Client::builder()
            .default_headers(headers)
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| Error::Network)?;
        Ok(Self {
            client,
            upload_client: upload_client()?,
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

pub(super) fn client_headers() -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
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
    Ok(headers)
}

const ACK_BULK_LIMIT: usize = 100;

const MESSAGE_REQUEST_PREVIEW_LIMIT: usize = 25;

const LOOKUP_BATCH: usize = 20;

const FRECENCY_SETTINGS: &str = "https://discord.com/api/v9/users/@me/settings-proto/2";
const PRELOADED_SETTINGS: &str = "https://discord.com/api/v9/users/@me/settings-proto/1";

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
    use super::messages::{
        forum_search_query, history_query, message_body, parse_uploads, put_request, reaction_url,
        upload_stream, uploads_body,
    };
    use super::profiles::profile_query;
    use super::*;
    use crate::discord::OutgoingReply;
    use std::sync::atomic::Ordering;
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
            upload_client: upload_client().unwrap(),
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

    async fn accept_upload(listener: &TcpListener) -> (TcpStream, String) {
        timeout(Duration::from_secs(2), async {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut bytes = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = stream.read(&mut bytes).await.unwrap();
                assert!(count > 0 && request.len() < 16384);
                request.extend_from_slice(&bytes[..count]);
            }
            let split = request
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
                .unwrap()
                + 4;
            let headers = String::from_utf8_lossy(&request[..split]).to_string();
            let length = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find_map(|(name, value)| {
                    if name.eq_ignore_ascii_case("content-length") {
                        value.trim().parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            while request.len() - split < length {
                let count = stream.read(&mut bytes).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&bytes[..count]);
            }
            (stream, headers)
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
            attachments: Vec::new(),
        }
    }

    fn upload_file(name: &str, size: u64, content_type: &str) -> UploadFile {
        UploadFile {
            path: std::path::PathBuf::from(name),
            filename: name.to_owned(),
            size,
            content_type: content_type.to_owned(),
            spoiler: false,
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
    fn send_payload_attachments_follow_flags() {
        let mut message = outgoing(None);
        message.attachments = vec![crate::discord::OutgoingAttachment {
            filename: "image.png".into(),
            uploaded_filename: "uuid/image.png".into(),
            content_type: "image/png".into(),
            spoiler: true,
        }];
        assert_eq!(
            serde_json::to_string(&message_body(None, 20, &message)).unwrap(),
            r#"{"mobile_network_type":"unknown","content":"hi","nonce":"123","tts":false,"flags":0,"attachments":[{"id":"0","description":"","filename":"image.png","uploaded_filename":"uuid/image.png","is_spoiler":true,"original_content_type":"image/png"}]}"#
        );
        message.attachments[0].spoiler = false;
        assert!(
            !serde_json::to_string(&message_body(None, 20, &message))
                .unwrap()
                .contains("is_spoiler")
        );
    }

    #[test]
    fn upload_delete_encodes_the_slash_as_captured() {
        assert_eq!(
            messages::delete_upload_url("uuid/Cargo.toml"),
            "https://discord.com/api/v9/attachments/uuid%2FCargo.toml"
        );
    }

    #[test]
    fn create_uploads_body_and_response_match_captured_traffic() {
        let files = [upload_file("image.png", 13726, "image/png")];
        assert_eq!(
            serde_json::to_string(&uploads_body(&files)).unwrap(),
            r#"{"files":[{"filename":"image.png","file_size":13726,"id":"0","is_clip":false,"original_content_type":"image/png"}]}"#
        );
        let response = r#"{"attachments":[{"id":0,"upload_url":"https://cdn.discordapp.com/attachments/1/2/uuid/image.png","upload_filename":"uuid/image.png"}]}"#;
        let targets = parse_uploads(response).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, 0);
        assert_eq!(
            targets[0].upload_url,
            "https://cdn.discordapp.com/attachments/1/2/uuid/image.png"
        );
        assert_eq!(targets[0].upload_filename, "uuid/image.png");
        assert!(matches!(parse_uploads("{}"), Err(Error::Decode)));
    }

    #[test]
    fn upload_request_adds_no_authorization() {
        let client = upload_client().unwrap();
        let file = upload_file("image.png", 13726, "image/png");
        let request = put_request(
            &client,
            "https://example.test/upload",
            &file,
            reqwest::Body::from(Vec::new()),
        )
        .build()
        .unwrap();
        assert!(request.headers().get(AUTHORIZATION).is_none());
        assert_eq!(
            request
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .unwrap(),
            "application/octet-stream"
        );
        assert_eq!(
            request
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .unwrap(),
            "13726"
        );
    }

    #[tokio::test]
    async fn upload_client_sends_no_account_authorization() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/upload", listener.local_addr().unwrap());
        let rest = Rest::new("secret-token").unwrap();
        let path = std::env::temp_dir().join(format!("zcord-upload-auth-{}", std::process::id()));
        tokio::fs::write(&path, b"hello").await.unwrap();
        let file = UploadFile {
            path: path.clone(),
            filename: "hello.txt".into(),
            size: 5,
            content_type: "text/plain".into(),
            spoiler: false,
        };
        let progress = UploadProgress::new(5);
        let client = rest.upload_client.clone();
        let server = async {
            let (mut stream, headers) = accept_upload(&listener).await;
            assert!(!headers.to_ascii_lowercase().contains("\r\nauthorization:"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            stream.shutdown().await.unwrap();
        };
        let transfer = put_upload(&client, &url, &file, &progress);
        timeout(Duration::from_secs(5), async {
            let ((), result) = tokio::join!(server, transfer);
            result.unwrap();
        })
        .await
        .unwrap();
        let _ = tokio::fs::remove_file(&path).await;
    }

    fn stalled_upload(
        name: &str,
    ) -> (
        std::path::PathBuf,
        UploadFile,
        std::sync::Arc<UploadProgress>,
    ) {
        let path = std::env::temp_dir().join(format!("zcord-{name}-{}", std::process::id()));
        std::fs::write(&path, b"hello").unwrap();
        let file = UploadFile {
            path: path.clone(),
            filename: "hello.txt".into(),
            size: 5,
            content_type: "text/plain".into(),
            spoiler: false,
        };
        (path, file, UploadProgress::new(5))
    }

    #[tokio::test]
    async fn stalled_upload_gives_up_and_cancels_without_a_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/upload", listener.local_addr().unwrap());
        let held = tokio::spawn(async move {
            let mut streams = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                streams.push(stream);
            }
        });
        let client = upload_client().unwrap();

        let (path, file, progress) = stalled_upload("upload-stall");
        let stalled = messages::put_upload_watched(
            &client,
            &url,
            &file,
            &progress,
            Duration::from_millis(100),
        );
        let result = timeout(Duration::from_secs(5), stalled).await.unwrap();
        assert!(matches!(result, Err(Error::Network)));
        let _ = std::fs::remove_file(&path);

        let (path, file, progress) = stalled_upload("upload-stall-cancel");
        progress.cancelled.store(true, Ordering::Release);
        let cancelled =
            messages::put_upload_watched(&client, &url, &file, &progress, Duration::from_secs(60));
        let result = timeout(Duration::from_secs(5), cancelled).await.unwrap();
        assert!(matches!(result, Err(Error::Cancelled)));
        let _ = std::fs::remove_file(&path);
        held.abort();
    }

    #[tokio::test]
    async fn upload_stream_reads_in_chunks_and_stops_when_cancelled() {
        use futures::StreamExt;
        let path = std::env::temp_dir().join(format!("zcord-upload-stream-{}", std::process::id()));
        let bytes = vec![7u8; 150_000];
        tokio::fs::write(&path, &bytes).await.unwrap();
        let progress = UploadProgress::new(bytes.len() as u64);
        let file = tokio::fs::File::open(&path).await.unwrap();
        let mut stream = Box::pin(upload_stream(file, progress.clone()));
        let mut seen = Vec::new();
        let mut chunks = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            assert!(chunk.len() <= 64 * 1024);
            seen.extend_from_slice(&chunk);
            chunks += 1;
        }
        assert_eq!(chunks, 3);
        assert_eq!(seen, bytes);
        assert_eq!(progress.sent.load(Ordering::Acquire), bytes.len() as u64);
        let file = tokio::fs::File::open(&path).await.unwrap();
        let cancelled = UploadProgress::new(bytes.len() as u64);
        cancelled.cancelled.store(true, Ordering::Release);
        let mut stream = Box::pin(upload_stream(file, cancelled));
        assert!(matches!(stream.next().await, Some(Err(Error::Cancelled))));
        let _ = tokio::fs::remove_file(&path).await;
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
