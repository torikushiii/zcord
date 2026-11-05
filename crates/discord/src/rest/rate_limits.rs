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
mod tests {
    use super::*;
    use serde_json::json;

    fn route(method: Method, path: &str) -> Route {
        Route::of(&Request::new(
            method,
            format!("https://discord.com/api/v9/{path}")
                .parse()
                .unwrap(),
        ))
    }

    fn headers(entries: &[(&'static str, &'static str)]) -> HeaderMap {
        entries
            .iter()
            .map(|(name, value)| {
                (
                    reqwest::header::HeaderName::from_static(name),
                    reqwest::header::HeaderValue::from_static(value),
                )
            })
            .collect()
    }

    #[test]
    fn routes_ignore_minor_ids_and_queries_but_keep_methods_and_major_ids() {
        let profile = route(Method::GET, "users/1/profile?type=popout&guild_id=4");
        assert_eq!(
            profile,
            route(Method::GET, "users/2/profile?type=modal&guild_id=5")
        );
        let channel = route(Method::GET, "channels/10/messages/11");
        assert_eq!(channel, route(Method::GET, "channels/10/messages/12"));
        assert_ne!(channel, route(Method::GET, "channels/20/messages/11"));
        assert_ne!(channel, route(Method::DELETE, "channels/10/messages/11"));
        assert_ne!(
            route(Method::GET, "guilds/10/members"),
            route(Method::GET, "guilds/20/members")
        );
        assert_eq!(
            profile,
            Route::of(&Request::new(
                Method::GET,
                "https://discord.com/api/users/1/profile".parse().unwrap()
            ))
        );
    }

    #[test]
    fn route_keys_do_not_retain_reaction_values_or_webhook_tokens() {
        let reaction = route(
            Method::PUT,
            "channels/10/messages/11/reactions/wave%3A20/@me",
        );
        assert_eq!(
            reaction,
            route(
                Method::PUT,
                "channels/10/messages/12/reactions/%F0%9F%91%8D/@me"
            )
        );
        assert_eq!(
            reaction.path,
            "channels/:id/messages/:id/reactions/:emoji/@me"
        );
        let webhook = route(Method::POST, "webhooks/10/synthetic-token");
        assert_eq!(webhook.path, "webhooks/:id/:token");
        assert_eq!(webhook, route(Method::POST, "webhooks/10/different-token"));
        assert_ne!(webhook, route(Method::POST, "webhooks/20/synthetic-token"));
    }

    #[test]
    fn independent_routes_run_concurrently_with_a_bounded_total() {
        let mut state = State::default();
        let now = Instant::now();
        let routes: Vec<_> = (0..=MAX_IN_FLIGHT)
            .map(|id| route(Method::GET, &format!("channels/{id}/messages")))
            .collect();
        for route in &routes[..MAX_IN_FLIGHT] {
            assert!(matches!(state.admit(route, now), Ok(Admission::Ready)));
        }
        assert!(matches!(
            state.admit(&routes[MAX_IN_FLIGHT], now),
            Ok(Admission::Wait(None))
        ));
        state.release(&routes[0]);
        assert!(matches!(
            state.admit(&routes[MAX_IN_FLIGHT], now),
            Ok(Admission::Ready)
        ));
    }

    #[test]
    fn same_route_waits_for_its_response_without_an_artificial_delay() {
        let mut state = State::default();
        let now = Instant::now();
        let route = route(Method::GET, "users/1/profile");
        assert!(matches!(state.admit(&route, now), Ok(Admission::Ready)));
        assert!(matches!(
            state.admit(&route, now),
            Ok(Admission::Wait(None))
        ));
        state.observe(
            &route,
            200,
            &headers(&[
                ("x-ratelimit-bucket", "profile"),
                ("x-ratelimit-remaining", "2"),
            ]),
            now,
        );
        state.release(&route);
        assert!(matches!(state.admit(&route, now), Ok(Admission::Ready)));
    }

    #[test]
    fn learned_buckets_share_limits_but_not_across_major_resources() {
        let mut state = State::default();
        let now = Instant::now();
        let get = route(Method::GET, "channels/10/messages");
        let post = route(Method::POST, "channels/10/messages");
        let other = route(Method::GET, "channels/20/messages");
        let bucket = headers(&[("x-ratelimit-bucket", "messages")]);
        for route in [&get, &post, &other] {
            assert!(matches!(state.admit(route, now), Ok(Admission::Ready)));
            state.observe(route, 200, &bucket, now);
        }
        let exhausted = headers(&[
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset-after", "1.25"),
        ]);
        state.observe(&get, 200, &exhausted, now);
        state.release(&get);
        state.release(&post);
        state.release(&other);
        let until = now + Duration::from_millis(1250);
        assert!(
            matches!(state.admit(&post, now), Ok(Admission::Wait(Some(deadline))) if deadline == until)
        );
        assert!(matches!(state.admit(&other, now), Ok(Admission::Ready)));
        assert!(matches!(state.admit(&get, until), Ok(Admission::Ready)));
    }

    #[test]
    fn global_backoff_blocks_all_routes_and_keeps_the_longest_deadline() {
        let mut state = State::default();
        let now = Instant::now();
        let profile = route(Method::GET, "users/1/profile");
        let outbox = route(Method::GET, "content-inventory/users/1/outbox");
        state.block(&profile, Duration::from_secs(2), true, now);
        state.block(&outbox, Duration::from_secs(1), true, now);
        assert!(
            matches!(state.admit(&outbox, now), Ok(Admission::Wait(Some(until))) if until == now + Duration::from_secs(2))
        );
        assert!(matches!(
            state.admit(&outbox, now + Duration::from_secs(2)),
            Ok(Admission::Ready)
        ));
    }

    #[test]
    fn retry_uses_the_longer_fractional_header_or_body_delay() {
        let limiter = Limiter::default();
        let profile = route(Method::GET, "users/1/profile");
        let permit = limiter.try_acquire(profile).unwrap().unwrap();
        let now = Instant::now();
        let delay = permit.retry(
            &headers(&[("retry-after", "0.25")]),
            &json!({"retry_after": 0.5, "global": true}),
        );
        assert_eq!(delay, Duration::from_millis(500));
        assert!(limiter.state().global_until.unwrap() >= now + delay);
        let outbox = route(Method::GET, "content-inventory/users/1/outbox");
        assert!(limiter.try_acquire(outbox).unwrap().is_none());
        drop(permit);
        assert_eq!(limiter.state().in_flight, 0);
    }

    #[tokio::test]
    async fn long_waits_fail_fast_instead_of_sleeping() {
        let limiter = Limiter::default();
        let route = route(Method::POST, "channels/10/messages");
        limiter
            .state()
            .block(&route, Duration::from_secs(60), false, Instant::now());
        let result = tokio::time::timeout(Duration::from_secs(1), limiter.acquire(route))
            .await
            .unwrap();
        assert!(matches!(
            result,
            Err(Error::RateLimited { retry_after }) if retry_after > Duration::from_secs(50)
        ));
    }

    #[test]
    fn global_headers_and_unusable_retry_delays_are_handled_conservatively() {
        for global in ["x-ratelimit-global", "x-ratelimit-scope"] {
            let limiter = Limiter::default();
            let permit = limiter
                .try_acquire(route(Method::GET, "users/1/profile"))
                .unwrap()
                .unwrap();
            let values = headers(&[
                (
                    global,
                    if global == "x-ratelimit-global" {
                        "true"
                    } else {
                        "global"
                    },
                ),
                ("retry-after", "NaN"),
            ]);
            assert_eq!(
                permit.retry(&values, &json!({"retry_after": -1})),
                FALLBACK_RETRY
            );
            assert!(limiter.state().global_until.is_some());
        }
    }

    #[test]
    fn exhausted_success_responses_use_reset_headers_or_a_fallback() {
        let mut state = State::default();
        let now = Instant::now();
        let route = route(Method::GET, "games");
        assert!(matches!(state.admit(&route, now), Ok(Admission::Ready)));
        state.observe(
            &route,
            200,
            &headers(&[("x-ratelimit-remaining", "0")]),
            now,
        );
        state.release(&route);
        assert!(
            matches!(state.admit(&route, now), Ok(Admission::Wait(Some(until))) if until == now + FALLBACK_RETRY)
        );
        let mut values = HeaderMap::new();
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            + 10.;
        values.insert("x-ratelimit-reset", epoch.to_string().parse().unwrap());
        let delay = reset_after(&values).unwrap();
        assert!(delay > Duration::from_secs(9) && delay <= Duration::from_secs(10));
        values.insert("x-ratelimit-reset-after", "0.25".parse().unwrap());
        assert_eq!(reset_after(&values), Some(Duration::from_millis(250)));
    }

    #[test]
    fn unauthorized_stops_future_requests_even_on_other_routes() {
        let limiter = Limiter::default();
        let permit = limiter
            .try_acquire(route(Method::GET, "users/1/profile"))
            .unwrap()
            .unwrap();
        permit.observe(401, &HeaderMap::new());
        assert!(matches!(
            limiter.try_acquire(route(Method::GET, "games")),
            Err(Error::Http(401))
        ));
    }

    #[tokio::test]
    async fn cancellation_does_not_leak_request_slots_or_miss_wakeups() {
        let limiter = Limiter::default();
        let profile = route(Method::GET, "users/1/profile");
        let first = limiter.acquire(profile.clone()).await.unwrap();
        {
            let canceled = limiter.acquire(profile.clone());
            tokio::pin!(canceled);
            assert!(futures::poll!(canceled).is_pending());
        }
        let waiting = limiter.acquire(profile.clone());
        tokio::pin!(waiting);
        assert!(futures::poll!(&mut waiting).is_pending());
        drop(first);
        let next = tokio::time::timeout(Duration::from_secs(1), &mut waiting)
            .await
            .unwrap()
            .unwrap();
        drop(next);
        assert_eq!(limiter.state().in_flight, 0);
        assert!(limiter.try_acquire(profile).unwrap().is_some());
    }

    #[test]
    fn pruning_retains_active_requests_and_cooldowns() {
        let mut state = State::default();
        let now = Instant::now();
        let active = route(Method::GET, "channels/1/messages");
        let blocked = route(Method::GET, "channels/2/messages");
        assert!(matches!(state.admit(&active, now), Ok(Admission::Ready)));
        state.block(&blocked, Duration::from_secs(60), false, now);
        for id in 3..MAX_TRACKED_ROUTES as u64 + 3 {
            let idle = route(Method::GET, &format!("channels/{id}/messages"));
            assert!(matches!(state.admit(&idle, now), Ok(Admission::Ready)));
            state.release(&idle);
        }
        assert!(state.windows.len() < MAX_TRACKED_ROUTES);
        assert!(matches!(
            state.admit(&active, now),
            Ok(Admission::Wait(None))
        ));
        assert!(matches!(
            state.admit(&blocked, now),
            Ok(Admission::Wait(Some(_)))
        ));
        state.release(&active);
        assert_eq!(state.in_flight, 0);
    }
}
