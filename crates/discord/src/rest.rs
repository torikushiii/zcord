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
use tokio::sync::OnceCell;
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

pub(super) struct Rest {
    client: Client,
    pub(super) upload_client: Client,
    limits: Limiter,
    installation_id: OnceCell<HeaderValue>,
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
            installation_id: OnceCell::new(),
        })
    }

    async fn send(&self, request: impl Fn() -> RequestBuilder) -> Result<Response, Error> {
        for attempt in 0..3 {
            let mut request = request().build().map_err(|_| Error::Network)?;
            if let Some(installation) = self.installation_id.get() {
                request
                    .headers_mut()
                    .insert("x-installation-id", installation.clone());
            }
            let permit = self.limits.acquire(Route::of(&request)).await?;
            request.headers_mut().insert(
                "x-super-properties",
                HeaderValue::from_str(&properties::super_properties())
                    .map_err(|_| Error::Decode)?,
            );
            match self.execute(request, permit).await {
                Err(Error::RateLimited { .. }) if attempt < 2 => continue,
                result => return result,
            }
        }
        unreachable!()
    }

    async fn fetch_installation(&self, url: &str, referer: &str) -> Result<&HeaderValue, Error> {
        self.installation_id
            .get_or_try_init(|| async {
                let response = self
                    .send(|| self.client.get(url).header(REFERER, referer))
                    .await?;
                let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
                let installation = value
                    .get("installation")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .ok_or(Error::Decode)?;
                let mut header = HeaderValue::from_str(installation).map_err(|_| Error::Decode)?;
                header.set_sensitive(true);
                Ok(header)
            })
            .await
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
mod tests;
