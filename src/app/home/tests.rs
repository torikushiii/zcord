use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{Context, IntoElement, Render, TestAppContext, Window, div, prelude::*, px};

use super::{
    ACTIVE_ART, ACTIVE_BADGE, active_art, active_detail_text, activity_elapsed, elapsed,
    request_age, request_preview_line, shows_elapsed_line,
};
use crate::{store::GameActivity, theme::rpx};

#[test]
fn elapsed_clock_requires_a_start_and_detail_row() {
    for (started_at, details, state, expected) in [
        (Some(UNIX_EPOCH), Some("Details"), None, true),
        (Some(UNIX_EPOCH), None, Some("State"), true),
        (Some(UNIX_EPOCH), None, None, false),
        (None, Some("Details"), None, false),
    ] {
        let game = GameActivity {
            name: "Game".into(),
            application_id: None,
            kind: 0,
            platform: None,
            embedded: false,
            playable: false,
            started_at,
            ends_at: None,
            details: details.map(str::to_owned),
            state: state.map(str::to_owned),
            large_image: None,
            small_image: None,
        };
        assert_eq!(shows_elapsed_line(&game), expected);
    }
}

#[test]
fn active_now_uses_server_time_and_preserves_header_duration() {
    let started = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let now = started + Duration::from_secs(12 * 60 + 27);
    let duration = activity_elapsed(Some(started), now).unwrap();
    assert_eq!(duration, Duration::from_secs(747));
    assert_eq!(elapsed(duration), "12m");
    assert_eq!(crate::app::voice::format_elapsed(duration), "12:27");
    assert_eq!(elapsed(Duration::from_secs(3600 + 5)), "1h");
    assert_eq!(
        crate::app::voice::format_elapsed(Duration::from_secs(3600 + 5)),
        "1:00:05"
    );
    assert_eq!(
        activity_elapsed(Some(started), now + Duration::from_secs(1)),
        Some(duration + Duration::from_secs(1))
    );
}

#[test]
fn missing_or_future_starts_do_not_fabricate_elapsed_time() {
    let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    assert_eq!(activity_elapsed(None, now), None);
    assert_eq!(
        activity_elapsed(Some(now + Duration::from_secs(1)), now),
        None
    );
    assert_eq!(activity_elapsed(Some(now), now), Some(Duration::ZERO));
}

struct DetailSample {
    width: f32,
    state: Option<&'static str>,
    started: Option<SystemTime>,
}

impl Render for DetailSample {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        div().w(px(self.width)).child(
            active_detail_text(
                42,
                "Views the home page",
                self.state,
                activity_elapsed(self.started, now),
            )
            .debug_selector(|| "active-detail-text".into()),
        )
    }
}

#[gpui::test]
fn elapsed_line_renders_with_or_without_state_without_overlapping(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| DetailSample {
        width: 220.,
        state: None,
        started: None,
    });
    let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    for width in [220., 120.] {
        for state in [None, Some("Lv.56|America")] {
            for started in [
                None,
                Some(now - Duration::from_secs(747)),
                Some(now + Duration::from_secs(1)),
            ] {
                view.update(cx, |sample, cx| {
                    sample.width = width;
                    sample.state = state;
                    sample.started = started;
                    cx.notify();
                });
                cx.update(|window, cx| {
                    let _ = window.draw(cx);
                });
                let elapsed = cx.debug_bounds("active-elapsed");
                let expected = activity_elapsed(started, now).is_some();
                assert_eq!(elapsed.is_some(), expected);
                if let Some(elapsed) = elapsed {
                    let text = cx.debug_bounds("active-detail-text").unwrap();
                    assert!(elapsed.top() >= text.top());
                    assert!(elapsed.bottom() <= text.bottom());
                    assert!(elapsed.right() <= text.right());
                }
            }
        }
    }
}

struct ArtSample {
    badge: bool,
}

impl Render for ArtSample {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let badge = self.badge.then(|| {
            div()
                .size_full()
                .debug_selector(|| "active-small-image".into())
                .into_any_element()
        });
        div()
            .flex()
            .items_center()
            .gap(rpx(12.))
            .child(
                active_art(div().size_full().into_any_element(), badge)
                    .debug_selector(|| "active-art".into()),
            )
            .child(
                div()
                    .debug_selector(|| "active-art-details".into())
                    .child("Details"),
            )
    }
}

#[gpui::test]
fn small_image_overlays_art_without_moving_or_overlapping_details(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| ArtSample { badge: false });
    let mut initial_details = None;
    for badge in [false, true] {
        view.update(cx, |sample, cx| {
            sample.badge = badge;
            cx.notify();
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let art = cx.debug_bounds("active-art").unwrap();
        let details = cx.debug_bounds("active-art-details").unwrap();
        assert_eq!(details, *initial_details.get_or_insert(details));
        let small = cx.debug_bounds("active-small-image");
        assert_eq!(small.is_some(), badge);
        if let Some(small) = small {
            assert_eq!(
                small.size.width,
                art.size.width * (ACTIVE_BADGE / ACTIVE_ART)
            );
            assert_eq!(small.size.height, small.size.width);
            assert!(small.left() > art.left());
            assert!(small.top() > art.top());
            assert!(small.right() > art.right());
            assert!(small.bottom() > art.bottom());
            assert!(small.right() < details.left());
        }
    }
}

#[test]
fn request_age_follows_discords_buckets() {
    let now = 1_800_000_000_000_i64;
    let ago = |seconds: i64| request_age(now - seconds * 1000, now);
    assert_eq!(ago(10), "1m ago");
    assert_eq!(ago(5 * 60), "5m ago");
    assert_eq!(ago(3 * 3_600), "3h ago");
    assert_eq!(ago(16 * 86_400), "16d ago");
    assert_eq!(ago(31 * 86_400), ">30d ago");
    assert_eq!(ago(359 * 86_400), ">30d ago");
    // A year or more falls back to the full date.
    assert!(ago(400 * 86_400).contains(", 20"));
}

#[test]
fn request_preview_flattens_lines_and_markup() {
    let store = crate::store::Store::seed();
    let line = request_preview_line(&store, "hi <:wave:123>\nthere <a:spin:9> <#55>");
    assert_eq!(line, "hi :wave: there :spin: <#55>");
    let mention = request_preview_line(&store, "<@999999> heya");
    assert!(mention.starts_with('@') && mention.ends_with(" heya"));
}
