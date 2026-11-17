use super::{
    Error, OutgoingMessage, ProfileKind, UploadFile, UploadProgress, UploadTarget, model,
    properties,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{
    Client, Method, Request, RequestBuilder, Response,
    header::{AUTHORIZATION, HeaderMap, HeaderValue, ORIGIN, REFERER, USER_AGENT},
};
use serde::Serialize;
use serde_json::Value;
use std::{collections::HashMap, time::Duration};
use tokio::time::sleep;
mod calls;
mod dms;
mod expressions;
mod gifs;
mod guilds;
mod inbox;
mod messages;
mod pins;
mod profile_styles;
mod profiles;
mod quests;
mod rate_limits;
mod requests;
mod saved;
mod settings;
mod shop;
mod streams;
mod threads;
pub(crate) use messages::{put_upload, upload_client};
use rate_limits::{Limiter, Permit, Route};

fn installation_header(id: &str) -> Option<HeaderValue> {
    let mut header = HeaderValue::from_str(id).ok().filter(|_| !id.is_empty())?;
    header.set_sensitive(true);
    Some(header)
}

const RETRY_STATUSES: [u16; 8] = [502, 504, 507, 522, 523, 524, 598, 599];
const MAX_FAILED_REQUESTS: u32 = 2;

enum Attempt {
    Done(Result<Response, Error>),
    Retry(Error),
    Backoff(Error),
}

pub(super) struct Rest {
    client: Client,
    pub(super) upload_client: Client,
    limits: Limiter,
    installation_id: std::sync::RwLock<Option<HeaderValue>>,
    authorization: std::sync::RwLock<HeaderValue>,
    view: std::sync::RwLock<(u64, u64)>,
}

fn authorization(token: &str) -> Result<HeaderValue, Error> {
    let mut authorization = HeaderValue::from_str(token).map_err(|_| Error::InvalidCredential)?;
    authorization.set_sensitive(true);
    Ok(authorization)
}

impl Rest {
    pub fn new(token: &str) -> Result<Self, Error> {
        let authorization = authorization(token)?;
        let client = Client::builder()
            .default_headers(client_headers()?)
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| Error::Network)?;
        Ok(Self {
            client,
            upload_client: upload_client()?,
            limits: Limiter::default(),
            installation_id: std::sync::RwLock::new(
                properties::installation_id().and_then(|id| installation_header(&id)),
            ),
            authorization: std::sync::RwLock::new(authorization),
            view: std::sync::RwLock::new((0, 0)),
        })
    }

    pub(super) fn set_token(&self, token: &str) -> Result<(), Error> {
        let value = authorization(token)?;
        *self
            .authorization
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = value;
        Ok(())
    }

    async fn send(&self, request: impl Fn() -> RequestBuilder) -> Result<Response, Error> {
        let mut retries = 0;
        let mut failed_requests = 0;
        loop {
            let mut request = request().build().map_err(|_| Error::Network)?;
            if failed_requests > 0 {
                request
                    .headers_mut()
                    .insert("x-failed-requests", HeaderValue::from(failed_requests));
            }
            self.prepare(&mut request);
            let permit = self.limits.acquire(Route::of(&request)).await?;
            request.headers_mut().insert(
                "x-super-properties",
                HeaderValue::from_str(&properties::super_properties())
                    .map_err(|_| Error::Decode)?,
            );
            let error = match self.execute(request, permit).await {
                Attempt::Done(result) => return result,
                Attempt::Retry(error) => error,
                Attempt::Backoff(error) => {
                    failed_requests += 1;
                    if retries < MAX_FAILED_REQUESTS {
                        sleep(Duration::from_secs(1 << retries)).await;
                    }
                    error
                }
            };
            if retries == MAX_FAILED_REQUESTS {
                return Err(error);
            }
            retries += 1;
        }
    }

    pub(super) fn set_view(&self, guild_id: u64, channel_id: u64) {
        let view = (guild_id, channel_id);
        if *self
            .view
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            != view
        {
            *self
                .view
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = view;
        }
    }

    fn view_referer(&self) -> String {
        match *self
            .view
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            (0, 0) => "https://discord.com/channels/@me".to_owned(),
            (0, channel_id) => referer(None, channel_id),
            (guild_id, 0) => format!("https://discord.com/channels/{guild_id}"),
            (guild_id, channel_id) => referer(Some(guild_id), channel_id),
        }
    }

    fn prepare(&self, request: &mut Request) {
        if !request.headers().contains_key(REFERER)
            && let Ok(referer) = HeaderValue::from_str(&self.view_referer())
        {
            request.headers_mut().insert(REFERER, referer);
        }
        let authorization = self
            .authorization
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        request.headers_mut().insert(AUTHORIZATION, authorization);
        request.headers_mut().insert(
            USER_AGENT,
            HeaderValue::from_static(properties::user_agent()),
        );
        if !matches!(*request.method(), Method::GET | Method::HEAD) {
            request
                .headers_mut()
                .entry(ORIGIN)
                .or_insert(HeaderValue::from_static("https://discord.com"));
        }
        if let Some(installation) = self.installation_id() {
            request
                .headers_mut()
                .insert("x-installation-id", installation);
        }
    }

    async fn try_send(&self, mut request: Request) -> Result<bool, Error> {
        self.prepare(&mut request);
        let Some(permit) = self.limits.try_acquire(Route::of(&request))? else {
            return Ok(false);
        };
        request.headers_mut().insert(
            "x-super-properties",
            HeaderValue::from_str(&properties::super_properties()).map_err(|_| Error::Decode)?,
        );
        match self.execute(request, permit).await {
            Attempt::Done(result) => result.map(|_| true),
            Attempt::Retry(error) | Attempt::Backoff(error) => Err(error),
        }
    }

    fn installation_id(&self) -> Option<HeaderValue> {
        self.installation_id
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(super) fn set_installation_id(&self, id: &str) -> bool {
        let Some(header) = installation_header(id) else {
            return false;
        };
        *self
            .installation_id
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(header);
        properties::set_installation_id(Some(id.to_owned()));
        true
    }

    async fn fetch_installation(&self, url: &str, referer: &str) -> Result<HeaderValue, Error> {
        if let Some(installation) = self.installation_id() {
            return Ok(installation);
        }
        let response = self
            .send(|| self.client.get(url).header(REFERER, referer))
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        let installation = value
            .get("installation")
            .and_then(Value::as_str)
            .ok_or(Error::Decode)?;
        if !self.set_installation_id(installation) {
            return Err(Error::Decode);
        }
        self.installation_id().ok_or(Error::Decode)
    }

    /// Only Retry-After makes a 429 or 503 safe to resend, and only reads are resent after a
    /// server error or a dropped connection.
    /// https://docs.discord.food/topics/rate-limits
    async fn execute(&self, request: Request, permit: Permit<'_>) -> Attempt {
        let idempotent = matches!(*request.method(), Method::GET | Method::HEAD);
        let response = match self.client.execute(request).await {
            Ok(response) => response,
            Err(_) if idempotent => return Attempt::Backoff(Error::Network),
            Err(_) => return Attempt::Done(Err(Error::Network)),
        };
        let status = response.status().as_u16();
        permit.observe(status, response.headers());
        if status == 401 {
            return Attempt::Done(Err(Error::Http(401)));
        }
        if status == 429 || status == 503 {
            let headers = response.headers().clone();
            let body = response.json::<Value>().await.unwrap_or(Value::Null);
            let error = Error::RateLimited {
                retry_after: permit.retry(&headers, &body),
            };
            return if headers.contains_key("retry-after") {
                Attempt::Retry(error)
            } else {
                Attempt::Done(Err(error))
            };
        }
        if !response.status().is_success() {
            let value = response.json::<Value>().await.unwrap_or(Value::Null);
            let error = classify(status, &value);
            return if idempotent && RETRY_STATUSES.contains(&status) {
                Attempt::Backoff(error)
            } else {
                Attempt::Done(Err(error))
            };
        }
        Attempt::Done(Ok(response))
    }

    pub(super) async fn external_assets(
        &self,
        application_id: u64,
        urls: &[String],
    ) -> Result<Value, Error> {
        let body = serde_json::json!({"urls": urls});
        let response = self
            .send(|| {
                self.client
                    .post(format!(
                        "https://discord.com/api/v9/applications/{application_id}/external-assets"
                    ))
                    .header(REFERER, "https://discord.com/channels/@me")
                    .json(&body)
            })
            .await?;
        response.json::<Value>().await.map_err(|_| Error::Decode)
    }

    pub(super) async fn application_assets(&self, application_id: u64) -> Result<Value, Error> {
        let response = self
            .send(|| {
                self.client
                    .get(format!(
                        "https://discord.com/api/v9/oauth2/applications/{application_id}/assets"
                    ))
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        response.json::<Value>().await.map_err(|_| Error::Decode)
    }
}

pub(super) fn client_headers() -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(properties::user_agent()),
    );
    for (name, value) in [
        ("accept", "*/*"),
        ("accept-language", properties::ACCEPT_LANGUAGE),
        ("priority", "u=1, i"),
        ("sec-ch-ua", properties::SEC_CH_UA),
        ("sec-ch-ua-mobile", "?0"),
        ("sec-ch-ua-platform", properties::sec_ch_ua_platform()),
        ("sec-fetch-dest", "empty"),
        ("sec-fetch-mode", "cors"),
        ("sec-fetch-site", "same-origin"),
        ("x-debug-options", "bugReporterEnabled"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
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
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'.' | b'_' | b'~' | b'!' | b'*' | b'\'' | b'(' | b')'
            )
        {
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
mod tests;
