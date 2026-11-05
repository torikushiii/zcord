use std::{cell::Cell, rc::Rc, sync::Arc, time::Instant};

use futures::channel::oneshot;
use gpui::{Anchor, TestAppContext, point, px};
use serde_json::json;

use super::{ProfileCard, ProfileOrigin, ProfileTheme, Tab};
use crate::{discord::model, media::images::ImageSpec, store::Store};

fn server_profile() -> Arc<model::Profile> {
    let mut value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../crates/discord/src/fixtures/profile_modal.json"
    ))
    .unwrap();
    value["user"]["global_name"] = json!("Main name");
    value["guild_member"]["nick"] = json!("Server name");
    value["guild_member"]["avatar"] = json!("server-avatar");
    value["guild_member"]["banner"] = json!("server-banner");
    value["guild_member"]["display_name_styles"] =
        json!({"font_id": 1, "effect_id": 2, "colors": [0x112233]});
    value["user_profile"]["theme_colors"] = json!([0x102030, 0x405060]);
    value["user_profile"]["pronouns"] = json!("main pronouns");
    value["user_profile"]["bio"] = json!("Main bio");
    value["user_profile"]["profile_effect"] = json!({"sku_id": "10"});
    value["user_profile"]["collectibles"] = json!([{"sku_id": "11", "type": 3}]);
    value["guild_member_profile"]["theme_colors"] = json!([0x708090, 0xA0B0C0]);
    value["guild_member_profile"]["pronouns"] = json!("server pronouns");
    value["guild_member_profile"]["bio"] = json!("Server bio");
    value["guild_member_profile"]["profile_effect"] = json!({"sku_id": "20"});
    value["guild_member_profile"]["collectibles"] = json!([{"sku_id": "21", "type": 3}]);
    Arc::new(serde_json::from_value(value).unwrap())
}

fn server_card() -> ProfileCard {
    let profile = server_profile();
    let mut card = ProfileCard::new(
        profile.user.user.id,
        Some(1),
        point(px(90.), px(140.)),
        Anchor::TopLeft,
        ProfileOrigin::Member,
    );
    card.profile = Some(profile);
    card
}

#[test]
fn only_profiles_opened_from_a_server_can_switch_scope() {
    let mut card = server_card();
    assert!(card.can_switch_scope());
    assert_eq!(card.guild_id, Some(1));
    assert_eq!(card.source_guild_id, Some(1));
    assert!(!card.more_menu_open);
    card.more_menu_open = true;
    assert!(card.switch_scope());
    assert!(card.can_switch_scope());
    assert_eq!(card.guild_id, None);
    assert_eq!(card.source_guild_id, Some(1));
    assert!(!card.more_menu_open);
    assert!(!card.modal);

    for origin in [
        ProfileOrigin::Member,
        ProfileOrigin::Account(Rc::new(Cell::new(None))),
    ] {
        let mut card = ProfileCard::new(2, None, point(px(0.), px(0.)), Anchor::TopLeft, origin);
        assert!(!card.can_switch_scope());
        assert_eq!(card.source_guild_id, None);
        assert!(!card.more_menu_open);
        card.more_menu_open = true;
        assert!(!card.switch_scope());
        assert_eq!(card.guild_id, None);
        assert!(!card.more_menu_open);
    }
}

#[test]
fn offline_round_trip_projects_main_fields_and_restores_the_server_snapshot() {
    let mut card = server_card();
    let server = card.profile.clone().unwrap();
    let main_banner = server.user.banner.clone().unwrap();
    assert!(matches!(
        super::popout::banner_spec(&card),
        Some(ImageSpec::GuildMemberBanner { guild_id: 1, hash, .. }) if hash == "server-banner"
    ));
    let mut store = Store::seed();
    store.absorb_profile(card.guild_id, &server);
    let global_avatar = store.user(card.user_id).unwrap().avatar.clone();
    let global_style = store.user(card.user_id).unwrap().name_style.clone();

    assert!(card.switch_scope());
    let main = card.profile.as_ref().unwrap();
    assert!(main.guild_member.is_none());
    assert!(main.guild_member_profile.is_none());
    assert!(server.guild_member.is_some());
    assert!(server.guild_member_profile.is_some());
    assert!(matches!(
        super::popout::banner_spec(&card),
        Some(ImageSpec::Banner { hash, .. }) if hash == main_banner
    ));
    let data = main.user_profile.as_ref().unwrap();
    assert_eq!(data.pronouns.as_deref(), Some("main pronouns"));
    assert_eq!(data.bio.as_deref(), Some("Main bio"));
    assert_eq!(data.profile_effect.as_ref().unwrap().sku_id, 10);
    assert_eq!(data.collectibles[0].sku_id, 11);
    assert_eq!(card.theme().unwrap().primary, 0x102030);
    assert_eq!(card.theme().unwrap().secondary, 0x405060);
    store.absorb_profile(card.guild_id, main);
    assert_eq!(store.member_name(0, card.user_id), Some("Main name"));
    assert_eq!(store.member_name(1, card.user_id), Some("Server name"));
    assert!(matches!(store.member_avatar_spec(1, card.user_id),
        Some(ImageSpec::GuildAvatar { hash, .. }) if hash == "server-avatar"));
    assert_eq!(store.user(card.user_id).unwrap().avatar, global_avatar);
    assert!(store.user(card.user_id).unwrap().name_style == global_style);
    assert_eq!(
        store.member_name_style(1, card.user_id).unwrap().font_id,
        Some(1)
    );

    assert!(card.switch_scope());
    assert_eq!(card.guild_id, Some(1));
    assert_eq!(card.source_guild_id, Some(1));
    assert!(Arc::ptr_eq(card.profile.as_ref().unwrap(), &server));
    assert_eq!(card.theme().unwrap().primary, 0x708090);
    let data = card
        .profile
        .as_ref()
        .unwrap()
        .guild_member_profile
        .as_ref()
        .unwrap();
    assert_eq!(data.pronouns.as_deref(), Some("server pronouns"));
    assert_eq!(data.bio.as_deref(), Some("Server bio"));
    assert_eq!(data.profile_effect.as_ref().unwrap().sku_id, 20);
    assert_eq!(data.collectibles[0].sku_id, 21);
}

#[test]
fn switching_preserves_the_surface_and_tab_but_resets_transient_scope_state() {
    let mut card = server_card();
    card.modal = true;
    card.tab = Tab::Friends;
    card.more_menu_open = true;
    card.shop_menu_open = true;
    card.hovered_badge = Some(1);
    card.hovered_cover = Some((2, 3));
    card.hovered_art = [true; 4];
    card.games_pending.insert(2);
    card.applications_pending.insert(3);
    card.widget_configs_pending.insert(4);
    card.cosmetics_pending.insert(20);
    card.board_requested = true;
    card.modal_request_pending = true;
    card.details_requested = true;
    card.wishlist_requested = true;
    card.note_requested = true;
    card.identities_pending = true;
    card.game_timer_started = true;
    card.effect_timer_started = true;
    card.avatar_theme = Some(ProfileTheme {
        primary: 1,
        secondary: 2,
    });
    card.avatar_theme_source = Some(ImageSpec::GuildAvatar {
        guild_id: 1,
        user_id: card.user_id,
        hash: "server-avatar".into(),
    });
    let product: Arc<model::CollectibleProduct> =
        Arc::new(serde_json::from_value(json!({"sku_id": "20"})).unwrap());
    card.effect = Some(product.clone());
    card.frame = Some(product);
    card.effect_started.replace(Some(Instant::now()));
    card.note = Some("A note".into());
    card.expanded_widgets.insert(2);
    card.widget_toggles.insert(2, (1, Instant::now()));
    let prompts = (card.status_prompt, card.activity_prompt);

    assert!(card.switch_scope());
    assert!(card.modal);
    assert!(card.tab == Tab::Friends);
    assert_eq!(card.position, point(px(90.), px(140.)));
    assert!(card.anchor == Anchor::TopLeft);
    assert!(matches!(card.origin, ProfileOrigin::Member));
    assert_eq!((card.status_prompt, card.activity_prompt), prompts);
    assert!(!card.more_menu_open && !card.shop_menu_open);
    assert!(card.hovered_badge.is_none() && card.hovered_cover.is_none());
    assert_eq!(card.hovered_art, [false; 4]);
    assert!(card.games_pending.is_empty() && card.applications_pending.is_empty());
    assert!(card.widget_configs_pending.is_empty() && card.cosmetics_pending.is_empty());
    assert!(!card.board_requested && !card.modal_request_pending && !card.details_requested);
    assert!(!card.wishlist_requested && !card.note_requested && !card.identities_pending);
    assert!(!card.game_timer_started && !card.effect_timer_started);
    assert!(card.avatar_theme.is_none() && card.avatar_theme_source.is_none());
    assert!(card.effect.is_none() && card.frame.is_none());
    assert!(card.effect_started.borrow().is_none());
    assert!(card.note.is_none() && card.badge_details.is_none());
    assert!(card.expanded_widgets.is_empty() && card.widget_toggles.is_empty());
}

#[test]
fn a_loaded_main_snapshot_is_kept_instead_of_reprojecting_old_server_data() {
    let mut card = server_card();
    assert!(card.switch_scope());
    let mut main = card.profile.as_ref().unwrap().as_ref().clone();
    main.user_profile.as_mut().unwrap().bio = Some("Updated main bio".into());
    let main = Arc::new(main);
    card.profile = Some(main.clone());
    assert!(card.switch_scope());
    assert!(card.switch_scope());
    assert!(Arc::ptr_eq(card.profile.as_ref().unwrap(), &main));
}

#[test]
fn a_seed_card_without_a_profile_response_still_switches_both_ways() {
    let mut card = ProfileCard::new(
        2,
        Some(1),
        point(px(0.), px(0.)),
        Anchor::TopLeft,
        ProfileOrigin::Member,
    );
    assert!(card.switch_scope());
    assert_eq!(card.guild_id, None);
    assert!(card.profile.is_none());
    assert!(card.switch_scope());
    assert_eq!(card.guild_id, Some(1));
    assert!(card.profile.is_none());
}

#[gpui::test]
fn switching_cancels_old_callbacks_even_after_switching_back(cx: &mut TestAppContext) {
    let mut card = server_card();
    let completed = Rc::new(Cell::new(false));
    let callback = completed.clone();
    let (send, receive) = oneshot::channel::<()>();
    card._tasks.push(cx.foreground_executor().spawn(async move {
        if receive.await.is_ok() {
            callback.set(true);
        }
    }));
    cx.run_until_parked();
    assert!(card.switch_scope());
    assert!(card.switch_scope());
    let _ = send.send(());
    cx.run_until_parked();
    assert!(
        !completed.get(),
        "an old scope's callback must stay cancelled"
    );
    assert!(card._tasks.is_empty());
}
