use super::{
    EffectLoad, THEME_OVERLAY, effect_finished, effect_playback, effect_start,
    loading::outbox_application_ids,
    modal::profile_close_button,
    palette::{ProfilePalette, ProfileTheme, blend, contrast},
    settle_effect_layers,
};
use crate::discord::model;
use gpui::rgb;
use std::time::{Duration, Instant};

#[test]
fn effect_layers_wait_for_pending_and_skip_failed() {
    use EffectLoad::{Failed, Pending, Ready};
    assert_eq!(settle_effect_layers([Ready(1), Pending]), None);
    assert_eq!(
        settle_effect_layers([Ready(1), Failed, Ready(2)]),
        Some(vec![1, 2])
    );
    assert_eq!(
        settle_effect_layers([Failed, Failed]),
        Some(Vec::<u8>::new())
    );

    let now = Instant::now();
    let mut started = None;
    assert_eq!(effect_start(&mut started, now), now);
    assert_eq!(
        effect_start(&mut started, now + Duration::from_secs(1)),
        now
    );
}

#[test]
fn profile_effect_intro_plays_once_and_only_the_loop_restarts() {
    let products: Vec<model::CollectibleProduct> = serde_json::from_str(include_str!(
        "../../../crates/discord/src/fixtures/collectibles_products.json"
    ))
    .unwrap();
    let effect = products.iter().find(|product| product.kind == 1).unwrap();
    let intro = &effect.items[0].effects[0];
    let repeat = &effect.items[0].effects[1];
    let started = Instant::now();
    let at = |ms| started + Duration::from_millis(ms);
    for ms in [0, 1000, 4980] {
        assert_eq!(effect_playback(intro, started, at(ms)), Some((0, started)));
        assert!(!effect_finished(intro, started, at(ms)));
    }
    for ms in [4981, 7981, 10_000, 15_961, 50_000] {
        assert_eq!(effect_playback(intro, started, at(ms)), None);
        assert!(effect_finished(intro, started, at(ms)));
        assert!(!effect_finished(repeat, started, at(ms)));
    }
    for ms in [0, 4981, 7980, 12_961, 15_960] {
        assert_eq!(effect_playback(repeat, started, at(ms)), None);
    }
    for ms in [7981, 10_000, 12_960] {
        assert_eq!(
            effect_playback(repeat, started, at(ms)),
            Some((0, at(7981)))
        );
    }
    for ms in [15_961, 16_000, 20_940] {
        assert_eq!(
            effect_playback(repeat, started, at(ms)),
            Some((1, at(15_961)))
        );
    }
    assert_eq!(effect_playback(repeat, started, at(20_941)), None);
    assert_eq!(effect_playback(intro, at(1), started), None);
}

#[test]
fn profile_effect_zero_duration_never_divides_by_zero() {
    let layer = model::EffectLayer {
        src: String::new(),
        looped: true,
        duration: 0,
        start: 100,
        loop_delay: 10,
        z_index: 0,
    };
    let started = Instant::now();
    let at = |ms| started + Duration::from_millis(ms);
    assert_eq!(effect_playback(&layer, started, at(99)), None);
    assert_eq!(
        effect_playback(&layer, started, at(100)),
        Some((0, at(100)))
    );
    assert_eq!(effect_playback(&layer, started, at(101)), None);
    assert_eq!(effect_playback(&layer, started, at(110)), None);
    assert_eq!(
        effect_playback(&layer, started, at(111)),
        Some((1, at(111)))
    );
}

#[test]
fn profile_effect_sakura_base_stays_mounted_while_the_overlay_repeats() {
    let base = model::EffectLayer {
        src: String::new(),
        looped: true,
        duration: 2880,
        start: 0,
        loop_delay: 0,
        z_index: 100,
    };
    let overlay = model::EffectLayer {
        start: 2880,
        loop_delay: 2880,
        z_index: 101,
        ..base.clone()
    };
    let started = Instant::now();
    let at = |ms| started + Duration::from_millis(ms);
    for ms in [0, 2879, 2880, 2999, 3000, 5760, 8640, 50_000] {
        assert_eq!(effect_playback(&base, started, at(ms)), Some((0, started)));
    }
    for ms in [0, 2879, 5760, 8639, 11_520] {
        assert_eq!(effect_playback(&overlay, started, at(ms)), None);
    }
    for ms in [2880, 3000, 5759] {
        assert_eq!(
            effect_playback(&overlay, started, at(ms)),
            Some((0, at(2880)))
        );
    }
    for ms in [8640, 10_000, 11_519] {
        assert_eq!(
            effect_playback(&overlay, started, at(ms)),
            Some((1, at(8640)))
        );
    }
    let delayed = model::EffectLayer { start: 100, ..base };
    assert_eq!(effect_playback(&delayed, started, at(99)), None);
    assert_eq!(
        effect_playback(&delayed, started, at(3000)),
        Some((0, at(100)))
    );
}

fn profile() -> model::Profile {
    serde_json::from_str(include_str!(
        "../../../crates/discord/src/fixtures/profile_modal.json"
    ))
    .unwrap()
}

#[test]
fn recent_application_requests_skip_duplicates_and_non_game_entries() {
    let outbox = serde_json::from_value(serde_json::json!({"entries": [
        {"id": "1", "extra": {"type": "played_game_extra", "application_id": "7"}},
        {"id": "2", "extra": {"type": "played_game_extra", "application_id": "7"}},
        {"id": "3", "extra": {"type": "played_game_extra"}},
        {"id": "4", "extra": {"type": "listened_session_extra", "application_id": "8"}},
        {"id": "5", "extra": {"type": "played_game_extra", "application_id": "9"}}
    ]}))
    .unwrap();
    assert_eq!(outbox_application_ids(&outbox), [7, 9]);
}

#[test]
fn profile_theme_uses_global_colors_when_the_guild_has_no_theme() {
    let profile = profile();
    let theme = ProfileTheme::of(Some(&profile)).unwrap();
    assert_eq!((theme.primary, theme.secondary), (11687486, 14261667));
}

#[test]
fn offline_user_keeps_cosmetics_and_theme_after_profile_loading() {
    let profile = profile();
    let ready = serde_json::from_str(include_str!(
        "../../../crates/discord/src/fixtures/ready.json"
    ))
    .unwrap();
    let mut store = crate::store::Store::from_ready(ready);
    store.absorb_profile(Some(1191168960364544011), &profile);
    let user = store.user(profile.user.user.id).unwrap();
    assert!(user.status == crate::store::Status::Offline);
    assert!(user.decoration.is_some());
    assert_eq!(
        user.name_style.as_ref().unwrap().colors,
        [16758138, 16749423, 16743544, 15756170]
    );
    assert_eq!(ProfileTheme::of(Some(&profile)).unwrap().primary, 11687486);
}

#[test]
fn complete_guild_theme_overrides_global_colors() {
    let mut profile = profile();
    profile.guild_member_profile.as_mut().unwrap().theme_colors = Some(vec![0x336699, 0x993366]);
    let theme = ProfileTheme::of(Some(&profile)).unwrap();
    assert_eq!((theme.primary, theme.secondary), (0x336699, 0x993366));
}

#[test]
fn incomplete_guild_theme_does_not_hide_the_global_theme() {
    let mut profile = profile();
    for colors in [vec![], vec![0x336699]] {
        profile.guild_member_profile.as_mut().unwrap().theme_colors = Some(colors);
        let theme = ProfileTheme::of(Some(&profile)).unwrap();
        assert_eq!((theme.primary, theme.secondary), (11687486, 14261667));
    }
    profile.user_profile.as_mut().unwrap().theme_colors = None;
    assert!(ProfileTheme::of(Some(&profile)).is_none());
    assert!(ProfileTheme::of(None).is_none());
}

#[test]
fn nitro_profiles_without_custom_colors_use_the_avatar_palette() {
    let mut profile = profile();
    profile.user_profile.as_mut().unwrap().theme_colors = None;
    let avatar = ProfileTheme {
        primary: 0xDCD2AF,
        secondary: 0x292722,
    };
    assert!(ProfileTheme::needs_avatar(Some(&profile)));
    let resolved = ProfileTheme::resolve(Some(&profile), Some(avatar)).unwrap();
    assert_eq!(
        (resolved.primary, resolved.secondary),
        (avatar.primary, avatar.secondary)
    );
    assert!(ProfilePalette::of(Some(resolved)).light);
    assert!(ProfileTheme::resolve(Some(&profile), None).is_none());
    for premium_type in [None, Some(0), Some(1), Some(3)] {
        profile.premium_type = premium_type;
        assert!(!ProfileTheme::needs_avatar(Some(&profile)));
        assert!(ProfileTheme::resolve(Some(&profile), Some(avatar)).is_none());
    }
}

#[test]
fn saved_colors_always_take_priority_over_an_avatar_palette() {
    let mut profile = profile();
    let avatar = ProfileTheme {
        primary: 0xDCD2AF,
        secondary: 0x292722,
    };
    let resolved = ProfileTheme::resolve(Some(&profile), Some(avatar)).unwrap();
    assert_eq!((resolved.primary, resolved.secondary), (11687486, 14261667));
    assert!(!ProfileTheme::needs_avatar(Some(&profile)));
    profile.guild_member_profile.as_mut().unwrap().theme_colors = Some(vec![0x112233, 0x445566]);
    let resolved = ProfileTheme::resolve(Some(&profile), Some(avatar)).unwrap();
    assert_eq!((resolved.primary, resolved.secondary), (0x112233, 0x445566));
}

#[test]
fn dialog_background_matches_discord_lab_midpoint_and_lightness_limits() {
    for (primary, secondary, expected) in [
        (0x336699, 0x993366, 0x1D141F),
        (0x339966, 0x993366, 0x1C1917),
        (0x8000FF, 0x00FFFF, 0x000B33),
        (0xA59BBC, 0x9387A9, 0xC9C3D5),
        (0x15223A, 0x30203B, 0x141221),
        (0x00FFFF, 0x8000FF, 0x99B0FF),
        (0, 0xFFFFFF, 0x191A19),
        (0xFFFFFF, 0, 0xCCCCCC),
    ] {
        let background = ProfileTheme { primary, secondary }.modal_background();
        let actual: gpui::Rgba = background
            .as_solid()
            .expect("dialog must not repeat the card gradient")
            .into();
        let expected = rgb(expected);
        for (actual, expected) in [
            (actual.r, expected.r),
            (actual.g, expected.g),
            (actual.b, expected.b),
        ] {
            assert!((actual - expected).abs() <= 1. / 255. + f32::EPSILON);
        }
    }
}

#[test]
fn secondary_brightness_does_not_change_the_primary_color_mode() {
    for primary in [0x8000FF, 0x00C000, 0xFF0000, 0x7F7F7F] {
        for secondary in [0, 0x00FFFF, 0xFFFFFF] {
            let theme = ProfileTheme { primary, secondary };
            assert!(!theme.is_light());
            assert_eq!(theme.overlay(), gpui::hsla(0., 0., 0., THEME_OVERLAY));
        }
    }
    for secondary in [0, 0x8000FF, 0xFFFFFF] {
        assert!(
            ProfileTheme {
                primary: 0x808080,
                secondary
            }
            .is_light()
        );
    }
}

#[test]
fn pale_profile_colors_use_a_light_overlay_instead_of_becoming_charcoal() {
    let theme = ProfileTheme {
        primary: 0xA59BBC,
        secondary: 0x9387A9,
    };
    assert!(theme.is_light());
    assert_eq!(theme.surface(), 0xDBD7E4);
    assert_eq!(theme.surface_color(theme.secondary), 0xD4CFDD);
    assert_eq!(theme.overlay(), gpui::hsla(0., 0., 1., THEME_OVERLAY));
}

#[test]
fn dark_profile_colors_keep_the_dark_overlay() {
    let theme = ProfileTheme {
        primary: 0x15223A,
        secondary: 0x30203B,
    };
    assert!(!theme.is_light());
    assert_eq!(theme.surface(), blend(theme.primary, 0, THEME_OVERLAY));
    assert_eq!(theme.overlay(), gpui::hsla(0., 0., 0., THEME_OVERLAY));
}

#[test]
fn profile_foreground_colors_follow_the_local_theme_not_the_app_theme() {
    let mut profile = profile();
    profile.user_profile.as_mut().unwrap().theme_colors = Some(vec![0xA59BBC, 0x9387A9]);
    let light = ProfilePalette::of(ProfileTheme::of(Some(&profile)));
    assert!(light.light);
    assert_eq!(light.strong, 0x18141F);
    profile.user_profile.as_mut().unwrap().theme_colors = Some(vec![0x15223A, 0x30203B]);
    let dark = ProfilePalette::of(ProfileTheme::of(Some(&profile)));
    assert!(!dark.light);
    assert_eq!(dark.strong, crate::theme::strong());
    profile.user_profile.as_mut().unwrap().theme_colors = Some(vec![0x8000FF, 0x00FFFF]);
    let saturated = ProfilePalette::of(ProfileTheme::of(Some(&profile)));
    assert!(!saturated.light);
    assert_eq!(saturated.strong, crate::theme::strong());
    let default = ProfilePalette::of(None);
    assert!(!default.light);
    assert_eq!(default.text, crate::theme::text());
}

#[test]
fn close_button_constructs_without_duplicate_hover_styles() {
    let mut profile = profile();
    for colors in [
        None,
        Some(vec![0xA59BBC, 0x9387A9]),
        Some(vec![0x15223A, 0x30203B]),
    ] {
        profile.user_profile.as_mut().unwrap().theme_colors = colors;
        let _button = profile_close_button(ProfilePalette::of(ProfileTheme::of(Some(&profile))));
    }
}

#[test]
fn profile_text_has_readable_contrast_at_both_gradient_stops() {
    let mut profile = profile();
    for colors in [
        [0xA59BBC, 0x9387A9],
        [0x15223A, 0x30203B],
        [0xFFFFFF, 0],
        [0xFFFF00, 0x0000FF],
        [0x8000FF, 0x00FFFF],
    ] {
        profile.user_profile.as_mut().unwrap().theme_colors = Some(colors.to_vec());
        let palette = ProfilePalette::of(ProfileTheme::of(Some(&profile)));
        let theme = ProfileTheme::of(Some(&profile)).unwrap();
        for background in colors.map(|color| theme.surface_color(color)) {
            for foreground in [palette.strong, palette.text, palette.muted, palette.faint] {
                assert!(contrast(foreground, background) >= 4.5);
            }
        }
    }
}
