use std::time::{Duration, UNIX_EPOCH};

use super::{
    active_now::{activity_elapsed, elapsed, shows_elapsed_line, voice_title},
    requests::{request_age, request_preview_line},
};
use crate::store::GameActivity;

#[test]
fn voice_cards_name_two_friends_and_count_everyone_else() {
    let names = |list: &[&str]| list.iter().map(|name| name.to_string()).collect::<Vec<_>>();
    assert_eq!(voice_title(&names(&["A"]), 1), "A");
    assert_eq!(voice_title(&names(&["A", "B"]), 2), "A and B");
    assert_eq!(voice_title(&names(&["A"]), 2), "A and 1 other");
    assert_eq!(voice_title(&names(&["A", "B"]), 3), "A, B, and 1 other");
    assert_eq!(voice_title(&names(&["A", "B"]), 5), "A, B, and 3 others");
}

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
    assert_eq!(crate::app::calls::voice::format_elapsed(duration), "12:27");
    assert_eq!(elapsed(Duration::from_secs(3600 + 5)), "1h");
    assert_eq!(
        crate::app::calls::voice::format_elapsed(Duration::from_secs(3600 + 5)),
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
