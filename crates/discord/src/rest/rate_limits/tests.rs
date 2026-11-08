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
