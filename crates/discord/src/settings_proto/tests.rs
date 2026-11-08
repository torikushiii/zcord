use super::*;

use crate::settings_fixtures::fixture;

#[test]
fn folders_decode_packed_unpacked_and_wrappers() {
    for packed in [true, false] {
        let folders = decode_base64(&fixture(packed)).unwrap().folders.unwrap();
        assert_eq!(folders.len(), 2);
        assert_eq!(folders[0].guild_ids, [100]);
        assert_eq!(folders[0].id, None);
        assert_eq!(
            folders[1],
            GuildFolder {
                guild_ids: vec![200, 999],
                id: Some(42),
                name: Some("Gaming".into()),
                color: Some(0x5865F2)
            }
        );
    }
    assert_eq!(decode(&[8, 0]).unwrap().folders, None);
    assert_eq!(decode(&[114, 0]).unwrap().folders, Some(Vec::new()));
}
#[test]
fn malformed_protobuf_is_rejected_without_panicking() {
    for bytes in [
        &[114, 10, 0][..],
        &[0xff; 11],
        &[0],
        &[14],
        &[11, 20],
        &[114, 3, 10, 1, 9],
    ] {
        assert!(decode(bytes).is_err());
    }
    let valid = STANDARD.decode(fixture(true)).unwrap();
    for length in 0..valid.len() {
        let _ = decode(&valid[..length]);
    }
    assert!(decode_base64("not base64!").is_err());
}

#[test]
fn status_presence_wrappers_and_expiry() {
    use crate::settings_fixtures::status_fixture;
    let decoded = decode_base64(&status_fixture(
        Some("invisible"),
        Some(("working", 0, "\u{1f3ae}", 0)),
    ))
    .unwrap();
    let status = decoded.status.unwrap();
    assert!(decoded.folders.is_none());
    assert_eq!(status.status.as_deref(), Some("invisible"));
    assert!(status.custom_status_present);
    assert_eq!(status.custom_status.unwrap().text, "working");
    let expired = decode_base64(&status_fixture(None, Some(("gone", 7, "x", 1))))
        .unwrap()
        .status
        .unwrap();
    assert_eq!(expired.status, None);
    assert!(expired.custom_status_present);
    assert!(expired.custom_status.is_none());
    let empty = decode_base64(&status_fixture(None, None))
        .unwrap()
        .status
        .unwrap();
    assert!(!empty.custom_status_present);
    assert!(decode(&[]).unwrap().status.is_none());
    let future = u64::MAX;
    assert!(
        decode_base64(&status_fixture(
            Some("idle"),
            Some(("future", 42, "custom", future))
        ))
        .unwrap()
        .status
        .unwrap()
        .custom_status
        .is_some()
    );
}

use crate::settings_fixtures::{field, uint};

#[test]
fn status_activity_sharing_decodes_absent_false_and_true() {
    let absent = decode(&field(11, &field(1, &field(1, b"online"))))
        .unwrap()
        .status
        .unwrap();
    assert_eq!(absent.show_current_game, None);

    let mut status = field(1, &field(1, b"online"));
    status.extend(field(3, &[]));
    let disabled = decode(&field(11, &status)).unwrap().status.unwrap();
    assert_eq!(disabled.show_current_game, Some(false));

    let mut status = field(1, &field(1, b"online"));
    status.extend(field(3, &uint(1, 1)));
    let enabled = decode(&field(11, &status)).unwrap().status.unwrap();
    assert_eq!(enabled.show_current_game, Some(true));
}

fn fixed64_field(number: u32, value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    push_varint(&mut bytes, (u64::from(number) << 3) | 1);
    bytes.extend_from_slice(&value.to_le_bytes());
    bytes
}

#[test]
fn manual_status_timing_is_distinct_from_custom_status_expiration() {
    let mut custom = field(1, b"keep custom");
    custom.extend(fixed64_field(4, u64::MAX));
    let mut status = field(1, &field(1, b"idle"));
    status.extend(field(2, &custom));
    status.extend(fixed64_field(4, 9_000));
    status.extend(field(5, &uint(1, 1_000)));
    let proto = field(11, &status);
    assert_eq!(
        status_timing(&proto).unwrap(),
        StatusTiming {
            expires_at_ms: 9_000,
            created_at_ms: Some(1_000),
        }
    );
    assert_eq!(
        decode(&proto)
            .unwrap()
            .status
            .unwrap()
            .custom_status
            .unwrap()
            .expires_at_ms,
        u64::MAX
    );
    assert_eq!(status_timing(&[]).unwrap(), StatusTiming::default());
}

#[test]
fn explicit_duration_replaces_manual_timing_and_preserves_unknown_wrapper_metadata() {
    let status_unknown = uint(77, 3);
    let created_unknown = field(88, b"creation-metadata");
    let mut wrapper = field(1, b"invisible");
    wrapper.extend(&status_unknown);
    let mut created = uint(1, 111);
    created.extend(&created_unknown);
    let mut custom = field(1, b"keep custom");
    custom.extend(fixed64_field(4, u64::MAX));
    let custom = field(2, &custom);
    let sharing = field(3, &uint(1, 0));
    let unknown = field(99, b"unknown-status-setting");
    let mut status = field(1, &wrapper);
    status.extend(&custom);
    status.extend(&sharing);
    status.extend(fixed64_field(4, 999));
    status.extend(field(5, &created));
    status.extend(&unknown);
    let mut base = field(11, &status);
    let unrelated = field(100, b"do not send or overwrite");
    base.extend(&unrelated);

    for (choice, expires, created, expected_expires, expected_created) in [
        ("dnd", 30_000, Some(2_000), 30_000, 2_000),
        ("idle", 0, None, 0, 0),
        ("online", 30_000, Some(2_000), 0, 0),
    ] {
        let patch = status_patch_at(&base, choice, expires, created).unwrap();
        let top = fields(&patch).unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].number, 11);
        let content = fields(top[0].value.unwrap()).unwrap();
        for preserved in [&custom, &sharing, &unknown] {
            assert!(
                content
                    .iter()
                    .any(|field| field.raw == preserved.as_slice())
            );
        }
        for (number, opaque) in [(1, &status_unknown), (5, &created_unknown)] {
            let wrapper = content.iter().find(|field| field.number == number).unwrap();
            assert!(
                fields(wrapper.value.unwrap())
                    .unwrap()
                    .iter()
                    .any(|field| field.raw == opaque.as_slice())
            );
        }
        let timing = status_timing(&patch).unwrap();
        assert_eq!(timing.expires_at_ms, expected_expires);
        assert_eq!(timing.created_at_ms, Some(expected_created));
        let expiry = content.iter().find(|field| field.number == 4).unwrap();
        assert_eq!(expiry.wire, 1, "manual expiry is fixed64");
        let merged = merge_partial(&base, &patch).unwrap();
        assert!(
            fields(&merged)
                .unwrap()
                .iter()
                .any(|field| field.raw == unrelated)
        );
    }
    let forever = status_patch(&base, "invisible").unwrap();
    assert_eq!(status_timing(&forever).unwrap().expires_at_ms, 0);
}

#[test]
fn malformed_manual_timing_fails_closed_without_touching_custom_timing() {
    for status in [uint(4, 123), field(5, &[8, 128]), vec![33, 1, 2]] {
        let proto = field(11, &status);
        assert!(status_timing(&proto).is_err());
        assert!(status_patch_at(&proto, "idle", 1_000, Some(1)).is_err());
    }
}

#[test]
fn status_patch_changes_only_the_status_wrapper_and_preserves_opaque_siblings() {
    let mut wrapper = field(1, b"invisible");
    wrapper.extend(uint(88, 7));
    let custom = field(2, &field(1, b"working"));
    let sharing = field(3, &uint(1, 0));
    let unknown = field(99, b"future-status-field");
    let mut status = field(1, &wrapper);
    status.extend(&custom);
    status.extend(&sharing);
    status.extend(&unknown);
    let unrelated = field(90, b"untouched");
    let mut base = field(1, &uint(3, 42));
    base.extend(field(11, &status));
    base.extend(&unrelated);
    for status in ["online", "idle", "dnd", "invisible"] {
        let patch = status_patch(&base, status).unwrap();
        let top = fields(&patch).unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].number, 11);
        let content = fields(top[0].value.unwrap()).unwrap();
        for preserved in [&custom, &sharing, &unknown] {
            assert!(
                content
                    .iter()
                    .any(|field| field.raw == preserved.as_slice())
            );
        }
        let wrapper = content.iter().find(|field| field.number == 1).unwrap();
        assert!(
            fields(wrapper.value.unwrap())
                .unwrap()
                .iter()
                .any(|field| field.raw == uint(88, 7))
        );
        let merged = merge_partial(&base, &patch).unwrap();
        assert_eq!(
            decode(&merged).unwrap().status.unwrap().status.as_deref(),
            Some(status)
        );
        assert_eq!(data_version(&merged).unwrap(), Some(42));
        assert!(
            fields(&merged)
                .unwrap()
                .iter()
                .any(|field| field.raw == unrelated)
        );
    }
}

#[test]
fn status_patch_and_version_fail_closed_on_malformed_input() {
    assert!(status_patch(&[90, 9, 0], "online").is_err());
    assert!(data_version(&[10, 4, 24]).is_err());
    assert_eq!(data_version(&[]).unwrap(), None);
    assert_eq!(data_version(&field(1, &[])).unwrap(), Some(0));
    assert_eq!(
        decode(&status_patch(&[], "invisible").unwrap())
            .unwrap()
            .status
            .unwrap()
            .status
            .as_deref(),
        Some("invisible")
    );
}

fn favorite(url: &str, src: &str, order: u32) -> FavoriteGif {
    FavoriteGif {
        url: url.into(),
        src: src.into(),
        format: 1,
        width: 200,
        height: 150,
        order,
    }
}

fn favorite_value(gif: &FavoriteGif) -> Vec<u8> {
    let mut value = uint(1, u64::from(gif.format));
    value.extend(field(2, gif.src.as_bytes()));
    value.extend(uint(3, u64::from(gif.width)));
    value.extend(uint(4, u64::from(gif.height)));
    value.extend(uint(5, u64::from(gif.order)));
    value
}

fn favorite_map_entry(gif: &FavoriteGif) -> Vec<u8> {
    let mut entry = field(1, gif.url.as_bytes());
    entry.extend(field(2, &favorite_value(gif)));
    entry
}

fn frecency_fixture(gifs: &[FavoriteGif]) -> Vec<u8> {
    let mut favorites = uint(2, 1);
    for gif in gifs {
        favorites.extend(field(1, &favorite_map_entry(gif)));
    }
    favorites.extend(field(9, b"unknown-subfield"));
    let mut bytes = field(1, &uint(1, 3));
    bytes.extend(field(2, &favorites));
    bytes.extend(field(5, &field(1, &[])));
    bytes.extend(field(6, &[8, 5]));
    bytes.extend(field(99, b"unknown-top-level"));
    bytes
}

#[test]
fn favorite_gifs_decode_and_sort_newest_first() {
    let older = favorite(
        "https://klipy.com/gifs/older",
        "https://static.klipy.com/older.webp",
        3,
    );
    let newer = favorite(
        "https://klipy.com/gifs/newer",
        "https://static.klipy.com/newer.webp",
        9,
    );
    let decoded = favorite_gifs(&frecency_fixture(&[newer.clone(), older.clone()])).unwrap();
    assert_eq!(decoded, [newer, older]);
    assert!(favorite_gifs(&[]).unwrap().is_empty());
    assert!(favorite_gifs(&frecency_fixture(&[])).unwrap().is_empty());
}

#[test]
fn with_favorite_gifs_replaces_only_map_entries() {
    let old = favorite(
        "https://klipy.com/gifs/old",
        "https://static.klipy.com/old.webp",
        1,
    );
    let first = favorite(
        "https://klipy.com/gifs/first",
        "https://static.klipy.com/first.webp",
        2,
    );
    let second = favorite(
        "https://klipy.com/gifs/second",
        "https://static.klipy.com/second.webp",
        7,
    );
    let original = frecency_fixture(&[old]);

    let mut expected_favorites = uint(2, 1);
    expected_favorites.extend(field(9, b"unknown-subfield"));
    expected_favorites.extend(field(1, &favorite_map_entry(&first)));
    expected_favorites.extend(field(1, &favorite_map_entry(&second)));
    let mut expected = field(1, &uint(1, 3));
    expected.extend(field(2, &expected_favorites));
    expected.extend(field(5, &field(1, &[])));
    expected.extend(field(6, &[8, 5]));
    expected.extend(field(99, b"unknown-top-level"));

    let updated = with_favorite_gifs(&original, &[first.clone(), second.clone()]).unwrap();
    assert_eq!(updated, expected);
    assert_eq!(favorite_gifs(&updated).unwrap(), [second, first]);

    let mut expected_favorites = uint(2, 1);
    expected_favorites.extend(field(9, b"unknown-subfield"));
    let mut expected_empty = field(1, &uint(1, 3));
    expected_empty.extend(field(2, &expected_favorites));
    expected_empty.extend(field(5, &field(1, &[])));
    expected_empty.extend(field(6, &[8, 5]));
    expected_empty.extend(field(99, b"unknown-top-level"));

    let removed = with_favorite_gifs(&updated, &[]).unwrap();
    assert_eq!(removed, expected_empty);
    assert!(favorite_gifs(&removed).unwrap().is_empty());
}

#[test]
fn with_favorite_gifs_creates_the_favorites_field_when_missing() {
    let gif = favorite(
        "https://klipy.com/gifs/new",
        "https://static.klipy.com/new.webp",
        1,
    );
    let bare = field(1, &uint(1, 3));
    let updated = with_favorite_gifs(&bare, std::slice::from_ref(&gif)).unwrap();
    assert_eq!(&updated[..bare.len()], &bare[..]);
    assert_eq!(favorite_gifs(&updated).unwrap(), [gif]);
}

#[test]
fn merge_partial_replaces_only_the_fields_carried_by_the_update() {
    let base = [field(1, b"one"), field(2, b"two"), field(5, b"five")].concat();
    let update = [field(2, b"new-two"), field(6, b"six"), uint(7, 9)].concat();
    assert_eq!(
        merge_partial(&base, &update).unwrap(),
        [
            field(1, b"one"),
            field(2, b"new-two"),
            field(5, b"five"),
            field(6, b"six"),
            uint(7, 9)
        ]
        .concat()
    );
    let repeated = [field(1, b"one"), field(2, b"a"), field(2, b"b")].concat();
    let replacement = [field(2, b"c"), field(2, b"d")].concat();
    assert_eq!(
        merge_partial(&repeated, &replacement).unwrap(),
        [field(1, b"one"), field(2, b"c"), field(2, b"d")].concat()
    );
    assert_eq!(merge_partial(&[], &[]).unwrap(), Vec::<u8>::new());
    assert_eq!(merge_partial(&base, &[]).unwrap(), base);
}

#[test]
fn malformed_frecency_input_is_rejected_without_panicking() {
    for bytes in [
        &[18][..],
        &[18, 5, 10][..],
        &[18, 3, 10, 1, b'x'][..],
        &[0xff; 11][..],
        &[0][..],
    ] {
        assert!(favorite_gifs(bytes).is_err());
    }
    for bytes in [&[18][..], &[18, 5, 10][..], &[0xff; 11][..], &[0][..]] {
        assert!(with_favorite_gifs(bytes, &[]).is_err());
    }
    let valid = frecency_fixture(&[]);
    for length in 0..valid.len() {
        let _ = favorite_gifs(&valid[..length]);
        let _ = with_favorite_gifs(&valid[..length], &[]);
        let _ = merge_partial(&valid[..length], &valid);
    }
    assert!(merge_partial(&valid, &[18, 9]).is_err());
}

#[test]
fn theme_decodes_every_choice_and_maps_absent_or_unset_to_none() {
    for (wire, expected) in [
        (1, AccountTheme::Dark),
        (2, AccountTheme::Light),
        (3, AccountTheme::Darker),
        (4, AccountTheme::Midnight),
    ] {
        assert_eq!(expected.wire(), wire);
        assert_eq!(AccountTheme::from_wire(wire), Some(expected));
        let decoded = decode(&field(13, &uint(1, wire))).unwrap();
        assert_eq!(decoded.theme, Some(expected));
    }
    assert_eq!(AccountTheme::from_wire(0), None);
    assert_eq!(AccountTheme::from_wire(5), None);
    assert_eq!(decode(&[]).unwrap().theme, None);
    assert_eq!(decode(&field(13, &uint(1, 0))).unwrap().theme, None);
    assert_eq!(decode(&field(13, &[])).unwrap().theme, None);
}

#[test]
fn theme_patch_replaces_the_theme_and_preserves_opaque_siblings() {
    let mut appearance = uint(1, 2);
    appearance.extend(uint(2, 1));
    appearance.extend(field(9, b"future-appearance-field"));
    let mut base = field(1, &uint(3, 42));
    base.extend(field(13, &appearance));
    base.extend(field(90, b"untouched"));

    let patch = theme_patch(&base, AccountTheme::Midnight).unwrap();
    let top = fields(&patch).unwrap();
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].number, 13);
    let content = fields(top[0].value.unwrap()).unwrap();
    assert!(content.iter().any(|entry| entry.raw == uint(2, 1)));
    assert!(
        content
            .iter()
            .any(|entry| entry.raw == field(9, b"future-appearance-field"))
    );
    assert!(!content.iter().any(|entry| entry.raw == uint(1, 2)));
    let merged = merge_partial(&base, &patch).unwrap();
    assert_eq!(decode(&merged).unwrap().theme, Some(AccountTheme::Midnight));
    assert_eq!(data_version(&merged).unwrap(), Some(42));
    assert!(
        fields(&merged)
            .unwrap()
            .iter()
            .any(|entry| entry.raw == field(90, b"untouched"))
    );
}

#[test]
fn theme_patch_creates_the_appearance_message_when_missing() {
    let base = field(1, &uint(3, 42));
    for theme in [
        AccountTheme::Dark,
        AccountTheme::Light,
        AccountTheme::Darker,
        AccountTheme::Midnight,
    ] {
        let patch = theme_patch(&base, theme).unwrap();
        assert_eq!(patch, field(13, &uint(1, theme.wire())));
        assert_eq!(decode(&patch).unwrap().theme, Some(theme));
    }
    assert!(theme_patch(&[106], AccountTheme::Dark).is_err());
    assert!(theme_patch(&field(13, &[8]), AccountTheme::Dark).is_err());
}
