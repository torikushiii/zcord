use super::{Error, model, properties};
use reqwest::{
    Client,
    header::{AUTHORIZATION, HeaderMap, HeaderValue, REFERER, USER_AGENT},
};
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
    ) -> Result<Vec<model::Message>, Error> {
        let referer = match guild_id {
            Some(guild_id) => format!("https://discord.com/channels/{guild_id}/{channel_id}"),
            None => format!("https://discord.com/channels/@me/{channel_id}"),
        };
        let mut gate = self.gate.lock().await;
        if gate.unauthorized {
            return Err(Error::Http(401));
        }
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/messages");
        for attempt in 0..3 {
            sleep_until(gate.next_request).await;
            gate.next_request = Instant::now() + Duration::from_secs(1);
            let mut query = vec![("limit", "50".to_owned())];
            if let Some(before) = before {
                query.push(("before", before.to_string()));
            }
            let response = self
                .client
                .get(&url)
                .query(&query)
                .header(REFERER, &referer)
                .send()
                .await
                .map_err(|_| Error::Network)?;
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
                return Err(Error::Http(status.as_u16()));
            }
            let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
            if !value.is_array() {
                return Err(Error::Decode);
            }
            let mut messages: Vec<model::Message> = model::parse_entries(value);
            messages.retain(|m| m.channel_id == channel_id);
            messages.sort_by_key(|m| m.id);
            messages.dedup_by_key(|m| m.id);
            return Ok(messages);
        }
        Err(Error::Decode)
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
    #[test]
    fn fractional_retry_after_is_seconds() {
        assert_eq!(seconds(Some(1.25)), Some(Duration::from_millis(1250)));
        assert_eq!(seconds(Some(f64::NAN)), None);
        assert_eq!(seconds(Some(-1.0)), None);
    }
}
