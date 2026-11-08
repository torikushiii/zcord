use super::{Presence, SelfStatus, StatusDuration};
use crate::{
    Error,
    settings_fixtures::{field, status_fixture, uint},
    settings_proto,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
use std::time::{Duration, UNIX_EPOCH};

fn snapshot(status: &str, version: u64) -> Vec<u8> {
    let mut proto = STANDARD
        .decode(status_fixture(
            Some(status),
            Some(("working", 42, "party", 0)),
        ))
        .unwrap();
    proto.extend(field(1, &uint(3, version)));
    proto
}

fn timed_snapshot(status: &str, version: u64, expires: u64, created: u64) -> Vec<u8> {
    let base = snapshot(status, version);
    let patch = settings_proto::status_patch_at(&base, status, expires, Some(created)).unwrap();
    settings_proto::merge_partial(&base, &patch).unwrap()
}

#[test]
fn duration_choices_match_reference_order_labels_and_checked_timestamp_math() {
    assert_eq!(
        StatusDuration::ALL.map(StatusDuration::label),
        [
            "For 15 Minutes",
            "For 1 Hour",
            "For 8 Hours",
            "For 24 Hours",
            "For 3 Days",
            "Forever",
        ]
    );
    let milliseconds = [900_000, 3_600_000, 28_800_000, 86_400_000, 259_200_000];
    for (choice, milliseconds) in StatusDuration::ALL.into_iter().zip(milliseconds) {
        assert_eq!(choice.deadline_ms(123).unwrap(), Some(123 + milliseconds));
        assert!(matches!(choice.deadline_ms(u64::MAX), Err(Error::Decode)));
    }
    assert_eq!(StatusDuration::Forever.deadline_ms(u64::MAX).unwrap(), None);
}

#[test]
fn manual_status_expires_exactly_to_online_without_clearing_custom_status() {
    for status in [
        SelfStatus::Idle,
        SelfStatus::DoNotDisturb,
        SelfStatus::Invisible,
    ] {
        let mut presence = Presence::default();
        presence
            .install(timed_snapshot(status.wire(), 1, 2_000, 1_000))
            .unwrap();
        let custom = presence.payload(1_999).unwrap()["d"]["activities"].clone();
        assert_eq!(custom.as_array().unwrap().len(), 1);
        assert_eq!(presence.status_at(1_999), Some(status));
        assert_eq!(
            presence.next_status_expiry_at(1_999),
            Some(UNIX_EPOCH + Duration::from_millis(2_000))
        );
        for now in [2_000, 2_001, u64::MAX] {
            assert_eq!(presence.status_at(now), Some(SelfStatus::Online));
            let payload = presence.payload(now).unwrap();
            assert_eq!(payload["d"]["status"], "online");
            assert_eq!(payload["d"]["since"], 0);
            assert_eq!(payload["d"]["activities"], custom);
            assert_eq!(presence.next_status_expiry_at(now), None);
        }
    }
}

#[test]
fn reconnect_keeps_absolute_deadline_and_newer_timing_supersedes_old_timer_state() {
    let original = timed_snapshot("idle", 1, 901_000, 1_000);
    let mut presence = Presence::default();
    presence.install(original.clone()).unwrap();
    let deadline = UNIX_EPOCH + Duration::from_millis(901_000);
    assert_eq!(presence.next_status_expiry_at(1_000), Some(deadline));
    presence.replace(Some(&STANDARD.encode(&original)));
    assert_eq!(presence.next_status_expiry_at(400_000), Some(deadline));
    assert_eq!(presence.status_at(901_000), Some(SelfStatus::Online));

    let revision = presence.revision;
    let newer = timed_snapshot("idle", 2, 1_500_000, 600_000);
    let mut partial =
        settings_proto::status_patch_at(&original, "idle", 1_500_000, Some(600_000)).unwrap();
    partial.extend(field(1, &uint(3, 2)));
    assert!(presence.update(&STANDARD.encode(&partial), true).unwrap());
    assert!(
        presence.revision > revision,
        "a timing-only edit invalidates older commands"
    );
    assert_eq!(presence.status_at(901_000), Some(SelfStatus::Idle));
    assert_eq!(presence.payload(901_000).unwrap()["d"]["status"], "idle");
    assert_eq!(
        presence.next_status_expiry_at(901_000),
        Some(UNIX_EPOCH + Duration::from_millis(1_500_000))
    );
    assert!(matches!(
        presence.install(original),
        Err(Error::SettingsOutdated)
    ));
    assert_eq!(presence.status_at(901_000), Some(SelfStatus::Idle));

    let clear = settings_proto::status_patch(&newer, "invisible").unwrap();
    let clear = settings_proto::merge_partial(&newer, &clear).unwrap();
    presence.install(clear).unwrap();
    assert_eq!(presence.status_at(u64::MAX), Some(SelfStatus::Invisible));
    assert_eq!(presence.next_status_expiry_at(0), None);
}

#[test]
fn only_a_known_non_online_future_manual_deadline_is_schedulable() {
    let mut presence = Presence::default();
    assert_eq!(presence.next_status_expiry_at(0), None);
    for (status, deadline) in [("idle", 0), ("idle", 1), ("unknown", 0), ("online", 3_000)] {
        presence.replace(Some(
            &STANDARD.encode(timed_snapshot(status, 1, deadline, 0)),
        ));
        assert_eq!(presence.next_status_expiry_at(2_000), None);
    }
    let mut base = STANDARD
        .decode(status_fixture(Some("dnd"), Some(("expired", 0, "", 1))))
        .unwrap();
    base.extend(field(1, &uint(3, 2)));
    let patch = settings_proto::status_patch_at(&base, "dnd", 3_000, Some(2_000)).unwrap();
    presence
        .install(settings_proto::merge_partial(&base, &patch).unwrap())
        .unwrap();
    assert_eq!(presence.status_at(2_000), Some(SelfStatus::DoNotDisturb));
    assert_eq!(
        presence.payload(2_000).unwrap()["d"]["activities"],
        json!([])
    );
    assert_eq!(
        presence.next_status_expiry_at(2_000),
        Some(UNIX_EPOCH + Duration::from_millis(3_000))
    );
}

#[test]
fn custom_activity_is_withdrawn_at_the_exact_expiry_boundary() {
    let mut presence = Presence::default();
    presence.replace(Some(&status_fixture(
        Some("invisible"),
        Some(("expiring", 0, "", u64::MAX)),
    )));
    assert_eq!(
        presence.payload(u64::MAX - 1).unwrap()["d"]["activities"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let expired = presence.payload(u64::MAX).unwrap();
    assert_eq!(expired["d"]["activities"], json!([]));
    assert_eq!(expired["d"]["status"], "invisible");
}

#[test]
fn choices_round_trip_and_invisible_is_never_sent_as_offline() {
    for status in SelfStatus::ALL {
        assert_eq!(SelfStatus::parse(status.wire()), Some(status));
        assert!(!status.label().is_empty());
        let mut presence = Presence::default();
        presence.install(snapshot(status.wire(), 1)).unwrap();
        let payload = presence.payload(1234).unwrap();
        assert_eq!(payload["op"], 3);
        assert_eq!(payload["d"]["status"], status.wire());
        assert_eq!(
            payload["d"]["since"],
            if status == SelfStatus::Idle { 1234 } else { 0 }
        );
        assert_eq!(payload["d"]["afk"], false);
        assert_eq!(
            payload["d"]["activities"],
            json!([{
                "name":"Custom Status","type":4,"state":"working",
                "emoji":{"id":"42","name":"party"}
            }])
        );
    }
    assert_eq!(SelfStatus::parse("offline"), Some(SelfStatus::Invisible));
    assert_eq!(SelfStatus::parse("unknown"), None);
}

#[test]
fn ready_unknown_does_not_force_online_and_reconnect_uses_current_settings() {
    let mut presence = Presence::default();
    presence.replace(Some(&status_fixture(Some("unknown"), None)));
    assert!(presence.payload(0).is_none());
    presence.replace(Some(&STANDARD.encode(snapshot("invisible", 3))));
    assert_eq!(presence.payload(0).unwrap()["d"]["status"], "invisible");
    let revision = presence.revision;
    assert!(
        presence
            .update(&STANDARD.encode(snapshot("dnd", 4)), false)
            .unwrap()
    );
    assert!(presence.revision > revision);
    assert!(matches!(
        presence.install(snapshot("online", 3)),
        Err(Error::SettingsOutdated)
    ));
    assert!(matches!(
        presence.update(&STANDARD.encode(snapshot("online", 3)), false),
        Err(Error::SettingsOutdated)
    ));
    assert_eq!(presence.status(), Some(SelfStatus::DoNotDisturb));
    presence.replace(Some(&STANDARD.encode(snapshot("idle", 1))));
    assert_eq!(presence.status(), Some(SelfStatus::Idle));
}

fn snapshot_with_game(show: Option<bool>) -> Vec<u8> {
    let mut custom = field(1, b"working");
    custom.push(17);
    custom.extend(42u64.to_le_bytes());
    custom.extend(field(3, b"party"));
    custom.push(33);
    custom.extend(0u64.to_le_bytes());
    let mut status = field(1, &field(1, b"online"));
    status.extend(field(2, &custom));
    if let Some(show) = show {
        let value = if show { uint(1, 1) } else { Vec::new() };
        status.extend(field(3, &value));
    }
    field(11, &status)
}

#[test]
fn local_activity_follows_custom_status_and_respects_account_privacy() {
    let mut presence = Presence::default();
    presence.install(snapshot_with_game(None)).unwrap();
    assert!(presence.shows_current_game());
    assert!(presence.set_local_activity(Some(json!({"type":0,"name":"Game"}))));
    let activities = presence.payload(0).unwrap()["d"]["activities"].clone();
    assert_eq!(activities[0]["name"], "Custom Status");
    assert_eq!(activities[1]["name"], "Game");

    let mut hidden = Presence::default();
    hidden.install(snapshot_with_game(Some(false))).unwrap();
    assert!(!hidden.shows_current_game());
    assert!(hidden.set_local_activity(Some(json!({"type":0,"name":"Game"}))));
    assert_eq!(
        hidden.payload(0).unwrap()["d"]["activities"],
        json!([{
            "name":"Custom Status","type":4,"state":"working",
            "emoji":{"id":"42","name":"party"}
        }])
    );

    let mut shown = Presence::default();
    shown.install(snapshot_with_game(Some(true))).unwrap();
    assert!(shown.shows_current_game());
}

#[test]
fn local_activity_revision_bumps_only_on_change() {
    let mut presence = Presence::default();
    presence.install(snapshot_with_game(None)).unwrap();
    let revision = presence.revision;
    let activity = json!({"type":0,"name":"Game"});
    assert!(presence.set_local_activity(Some(activity.clone())));
    assert_eq!(presence.revision, revision.wrapping_add(1));
    assert!(!presence.set_local_activity(Some(activity.clone())));
    assert_eq!(presence.revision, revision.wrapping_add(1));
    assert!(presence.set_local_activity(None));
    assert_eq!(presence.revision, revision.wrapping_add(2));
    assert!(!presence.set_local_activity(None));
    assert_eq!(presence.revision, revision.wrapping_add(2));
}

#[test]
fn activity_change_during_a_status_write_does_not_look_outdated() {
    let mut presence = Presence::default();
    presence.install(snapshot_with_game(None)).unwrap();
    let settings_revision = presence.settings_revision;
    assert!(presence.set_local_activity(Some(json!({"type":0,"name":"Game"}))));
    assert_eq!(presence.settings_revision, settings_revision);
    assert_ne!(presence.revision, settings_revision);
}

#[test]
fn unrelated_or_malformed_settings_do_not_publish_presence_or_erase_custom_status() {
    let mut presence = Presence::default();
    presence.install(snapshot("invisible", 1)).unwrap();
    let expected = presence.payload(0);
    let revision = presence.revision;
    assert!(
        !presence
            .update(&STANDARD.encode(field(99, b"unrelated")), true)
            .unwrap()
    );
    assert_eq!(presence.revision, revision);
    assert_eq!(presence.payload(0), expected);
    assert!(presence.update("not base64", true).is_err());
    assert_eq!(presence.payload(0), expected);
    presence.replace(Some(&status_fixture(
        Some("online"),
        Some(("expired", 0, "", 1)),
    )));
    assert_eq!(presence.payload(0).unwrap()["d"]["activities"], json!([]));
}
