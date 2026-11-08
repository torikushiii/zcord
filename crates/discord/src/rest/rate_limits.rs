use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard, PoisonError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::{Method, Request, header::HeaderMap};
use tokio::{
    sync::Notify,
    time::{Instant, sleep_until},
};

use super::{Error, seconds};

const MAX_IN_FLIGHT: usize = 4;
const MAX_TRACKED_ROUTES: usize = 1024;
const FALLBACK_RETRY: Duration = Duration::from_secs(5);
const MAX_WAIT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Route {
    method: Method,
    path: String,
    major: Major,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Major {
    None,
    Channel(u64),
    Guild(u64),
    Webhook(u64),
}

impl Route {
    pub fn of(request: &Request) -> Self {
        let segments: Vec<_> = request
            .url()
            .path_segments()
            .into_iter()
            .flatten()
            .collect();
        let segments = if segments.first() == Some(&"api") {
            let versioned = segments.get(1).is_some_and(|segment| {
                segment.strip_prefix('v').is_some_and(|version| {
                    !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit())
                })
            });
            &segments[if versioned { 2 } else { 1 }..]
        } else {
            segments.as_slice()
        };
        let major = match (
            segments.first().copied(),
            segments.get(1).and_then(|id| id.parse().ok()),
        ) {
            (Some("channels"), Some(id)) => Major::Channel(id),
            (Some("guilds"), Some(id)) => Major::Guild(id),
            (Some("webhooks"), Some(id)) => Major::Webhook(id),
            _ => Major::None,
        };
        let path = segments
            .iter()
            .enumerate()
            .map(|(index, segment)| {
                if segments.first() == Some(&"webhooks") && index == 2 {
                    ":token"
                } else if index > 0 && segments[index - 1] == "reactions" {
                    ":emoji"
                } else if !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit()) {
                    ":id"
                } else {
                    segment
                }
            })
            .collect::<Vec<_>>()
            .join("/");
        Self {
            method: request.method().clone(),
            path,
            major,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Bucket {
    Route(Route),
    Shared { id: String, major: Major },
}

#[derive(Default)]
struct Window {
    in_flight: usize,
    until: Option<Instant>,
}

#[derive(Default)]
struct State {
    aliases: HashMap<Route, Bucket>,
    windows: HashMap<Bucket, Window>,
    in_flight: usize,
    global_until: Option<Instant>,
    unauthorized: bool,
}

enum Admission {
    Ready,
    Wait(Option<Instant>),
}

impl State {
    fn bucket(&self, route: &Route) -> Bucket {
        self.aliases
            .get(route)
            .cloned()
            .unwrap_or_else(|| Bucket::Route(route.clone()))
    }

    fn admit(&mut self, route: &Route, now: Instant) -> Result<Admission, Error> {
        if self.unauthorized {
            return Err(Error::Http(401));
        }
        let bucket = self.bucket(route);
        let window = self.windows.entry(bucket).or_default();
        let until = self
            .global_until
            .into_iter()
            .chain(window.until)
            .filter(|until| *until > now)
            .max();
        if until.is_some() || window.in_flight > 0 || self.in_flight >= MAX_IN_FLIGHT {
            return Ok(Admission::Wait(until));
        }
        window.in_flight += 1;
        self.in_flight += 1;
        Ok(Admission::Ready)
    }

    fn learn(&mut self, route: &Route, id: &str) -> Bucket {
        let old = self.bucket(route);
        let bucket = Bucket::Shared {
            id: id.to_owned(),
            major: route.major,
        };
        if old != bucket {
            let previous = self
                .windows
                .get_mut(&old)
                .expect("request has a rate-limit window");
            previous.in_flight -= 1;
            let until = previous.until;
            let window = self.windows.entry(bucket.clone()).or_default();
            window.in_flight += 1;
            window.until = window.until.max(until);
            self.aliases.insert(route.clone(), bucket.clone());
            if matches!(old, Bucket::Route(_)) {
                self.windows.remove(&old);
            }
        }
        bucket
    }

    fn observe(&mut self, route: &Route, status: u16, headers: &HeaderMap, now: Instant) {
        let bucket_id = headers
            .get("x-ratelimit-bucket")
            .and_then(|value| value.to_str().ok())
            .filter(|id| !id.is_empty());
        let bucket = match bucket_id {
            Some(id) => self.learn(route, id),
            None => self.bucket(route),
        };
        if status == 401 {
            self.unauthorized = true;
        }
        let retry = header_seconds(headers, "retry-after");
        let exhausted = headers
            .get("x-ratelimit-remaining")
            .and_then(|value| value.to_str().ok())
            == Some("0");
        let reset = exhausted
            .then(|| reset_after(headers).or_else(|| (status != 429).then_some(FALLBACK_RETRY)))
            .flatten();
        if let Some(delay) = retry.into_iter().chain(reset).max() {
            let window = self.windows.entry(bucket).or_default();
            window.until = window.until.max(Some(now + delay));
        }
        if status == 429
            && is_global(headers)
            && let Some(retry) = retry
        {
            let until = now + retry;
            self.global_until = self.global_until.max(Some(until));
        }
    }

    fn block(&mut self, route: &Route, delay: Duration, global: bool, now: Instant) {
        let until = Some(now + delay);
        if global {
            self.global_until = self.global_until.max(until);
        } else {
            let bucket = self.bucket(route);
            let window = self.windows.entry(bucket).or_default();
            window.until = window.until.max(until);
        }
    }

    fn release(&mut self, route: &Route) {
        let bucket = self.bucket(route);
        if let Some(window) = self.windows.get_mut(&bucket) {
            window.in_flight -= 1;
        }
        self.in_flight -= 1;
        if self.windows.len() >= MAX_TRACKED_ROUTES || self.aliases.len() >= MAX_TRACKED_ROUTES {
            let now = Instant::now();
            self.windows.retain(|_, window| {
                window.in_flight > 0 || window.until.is_some_and(|until| until > now)
            });
            self.aliases
                .retain(|_, bucket| self.windows.contains_key(bucket));
        }
    }
}

#[derive(Default)]
pub(super) struct Limiter {
    state: Mutex<State>,
    changed: Notify,
}

impl Limiter {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub async fn acquire(&self, route: Route) -> Result<Permit<'_>, Error> {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let admission = self.state().admit(&route, Instant::now())?;
            match admission {
                Admission::Ready => {
                    return Ok(Permit {
                        limiter: self,
                        route,
                    });
                }
                Admission::Wait(Some(until)) => {
                    let retry_after = until.saturating_duration_since(Instant::now());
                    if retry_after > MAX_WAIT {
                        return Err(Error::RateLimited { retry_after });
                    }
                    tokio::select! {
                        _ = sleep_until(until) => {},
                        _ = notified => {},
                    }
                }
                Admission::Wait(None) => notified.await,
            }
        }
    }

    pub fn try_acquire(&self, route: Route) -> Result<Option<Permit<'_>>, Error> {
        Ok(match self.state().admit(&route, Instant::now())? {
            Admission::Ready => Some(Permit {
                limiter: self,
                route,
            }),
            Admission::Wait(_) => None,
        })
    }
}

pub(super) struct Permit<'a> {
    limiter: &'a Limiter,
    route: Route,
}

impl Permit<'_> {
    pub fn observe(&self, status: u16, headers: &HeaderMap) {
        self.limiter
            .state()
            .observe(&self.route, status, headers, Instant::now());
        self.limiter.changed.notify_waiters();
    }

    pub fn retry(&self, headers: &HeaderMap, body: &serde_json::Value) -> Duration {
        let delay = header_seconds(headers, "retry-after")
            .into_iter()
            .chain(seconds(
                body.get("retry_after").and_then(serde_json::Value::as_f64),
            ))
            .max()
            .unwrap_or(FALLBACK_RETRY)
            .max(Duration::from_millis(10));
        self.limiter.state().block(
            &self.route,
            delay,
            is_global(headers)
                || body.get("global").and_then(serde_json::Value::as_bool) == Some(true),
            Instant::now(),
        );
        self.limiter.changed.notify_waiters();
        delay
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.limiter.state().release(&self.route);
        self.limiter.changed.notify_waiters();
    }
}

fn header_seconds(headers: &HeaderMap, name: &str) -> Option<Duration> {
    seconds(
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok()),
    )
}

fn reset_after(headers: &HeaderMap) -> Option<Duration> {
    header_seconds(headers, "x-ratelimit-reset-after").or_else(|| {
        let reset: f64 = headers
            .get("x-ratelimit-reset")?
            .to_str()
            .ok()?
            .parse()
            .ok()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_secs_f64();
        seconds(Some(reset - now))
    })
}

fn is_global(headers: &HeaderMap) -> bool {
    headers
        .get("x-ratelimit-global")
        .and_then(|value| value.to_str().ok())
        == Some("true")
        || headers
            .get("x-ratelimit-scope")
            .and_then(|value| value.to_str().ok())
            == Some("global")
}

#[cfg(test)]
mod tests;
