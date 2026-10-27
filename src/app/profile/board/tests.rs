use std::{collections::HashMap, sync::Arc, time::Duration};

use gpui::{Context, IntoElement, Render, Styled, TestAppContext, Window, div, prelude::*, px};

use super::{
    COVER_GAP, COVER_HEIGHT, COVER_WIDTH, ProfilePalette, SkuPrice,
    actions::{GameActions, game_launch_url},
    activity::{activity_traits, clock, current_entry, parse, snowflake_time, span},
    widgets::{game_cover, game_cover_grid, stat_name, tag_label},
    wishlist::sku_price,
};
use crate::{
    discord::model::{Game, OutboxEntry, OutboxKind, OutboxTrait, PublicApplication},
    store::GameActivity,
    theme::{self, rpx},
};

#[test]
fn game_actions_use_each_applications_own_targets() {
    for (id, steam_id) in [(7, "480"), (8, "481")] {
        let application: PublicApplication = serde_json::from_value(serde_json::json!({
            "id": id,
            "name": "Game",
            "deeplink_uri": format!("steam://run/{steam_id}"),
            "third_party_skus": [{"distributor": "steam", "id": steam_id}]
        }))
        .unwrap();
        let applications = HashMap::from([(id, Arc::new(application))]);
        assert_eq!(
            GameActions::resolve(id, &HashMap::new(), &applications),
            GameActions {
                steam_url: Some(format!("https://store.steampowered.com/app/{steam_id}")),
                play_url: Some(format!("steam://run/{steam_id}")),
            }
        );
    }
}

#[test]
fn partner_widget_and_playing_game_share_linked_actions_without_presence_flags() {
    let application: PublicApplication = serde_json::from_value(serde_json::json!({
        "id": "7",
        "name": "Partner App",
        "deeplink_uri": "https://games.example/play",
        "linked_games": [{
            "id": "42",
            "application": {
                "id": "42", "name": "Game",
                "websites": [{"category": 1, "url": "https://games.example/play"}],
                "third_party_skus": [{"distributor": "steam", "id": "480"}]
            }
        }]
    }))
    .unwrap();
    let applications = HashMap::from([(7, Arc::new(application))]);
    let widget_actions = GameActions::resolve(7, &HashMap::new(), &applications);
    assert_eq!(
        widget_actions,
        GameActions {
            steam_url: Some("https://store.steampowered.com/app/480".into()),
            play_url: Some("https://games.example/play".into()),
        }
    );
    assert_eq!(
        GameActions::resolve(42, &HashMap::new(), &applications),
        widget_actions
    );
}

#[test]
fn game_actions_resolve_missing_embedded_metadata_from_loaded_games() {
    let application: PublicApplication = serde_json::from_value(serde_json::json!({
        "id": "7", "name": "Partner App",
        "deeplink_uri": "https://games.example/play",
        "linked_games": [{"id": "42"}]
    }))
    .unwrap();
    let game: Game = serde_json::from_value(serde_json::json!({
        "id": "42", "name": "Game",
        "third_party_skus": [{"distributor": "steam", "id": "480"}],
        "websites": [{"category": 1, "url": "https://games.example/play"}],
        "linked_applications": [{"id": "7"}]
    }))
    .unwrap();
    let mut applications = HashMap::from([(7, Arc::new(application))]);
    assert_eq!(
        GameActions::resolve(7, &HashMap::new(), &applications),
        GameActions::default()
    );
    let games = HashMap::from([(42, Arc::new(game))]);
    let expected = GameActions {
        steam_url: Some("https://store.steampowered.com/app/480".into()),
        play_url: Some("https://games.example/play".into()),
    };
    assert_eq!(GameActions::resolve(7, &games, &applications), expected);
    Arc::make_mut(applications.get_mut(&7).unwrap())
        .linked_games
        .clear();
    assert_eq!(GameActions::resolve(7, &games, &applications), expected);
    assert_eq!(GameActions::resolve(42, &games, &applications), expected);
}

#[test]
fn game_actions_prefer_official_website_to_campaign_and_store_links() {
    let application: PublicApplication = serde_json::from_value(serde_json::json!({
        "id": "7", "name": "Partner App",
        "deeplink_uri": "https://games.example/campaign/index.html",
        "linked_games": [{"id": "42"}]
    }))
    .unwrap();
    let game: Game = serde_json::from_value(serde_json::json!({
        "id": "42", "name": "Game",
        "websites": [
            {"category": 13, "url": "https://store.steampowered.com/app/480/"},
            {"category": 4, "url": "https://www.facebook.com/example"},
            {"category": 1, "url": "https://games.example/en/"}
        ],
        "linked_applications": [{"id": "7"}]
    }))
    .unwrap();
    let applications = HashMap::from([(7, Arc::new(application))]);
    let games = HashMap::from([(42, Arc::new(game))]);
    for id in [7, 42] {
        assert_eq!(
            GameActions::resolve(id, &games, &applications)
                .play_url
                .as_deref(),
            Some("https://games.example/en/")
        );
    }
}

#[test]
fn game_actions_do_not_use_a_campaign_when_the_official_site_is_missing() {
    let application: PublicApplication = serde_json::from_value(serde_json::json!({
        "id": "7", "name": "Partner App",
        "deeplink_uri": "https://games.example/campaign/index.html",
        "linked_games": [{
            "id": "42",
            "application": {
                "id": "42", "name": "Game",
                "websites": [{"category": 4, "url": "https://www.facebook.com/example"}]
            }
        }]
    }))
    .unwrap();
    let mut applications = HashMap::from([(7, Arc::new(application))]);
    assert_eq!(
        GameActions::resolve(7, &HashMap::new(), &applications).play_url,
        None
    );
    let linked_game = Arc::make_mut(applications.get_mut(&7).unwrap()).linked_games[0]
        .application
        .as_mut()
        .unwrap();
    linked_game
        .websites
        .push(crate::discord::model::GameWebsite {
            url: "http://games.example/".into(),
            category: 1,
        });
    assert_eq!(
        GameActions::resolve(7, &HashMap::new(), &applications)
            .play_url
            .as_deref(),
        Some("http://games.example/")
    );
}

#[test]
fn game_actions_do_not_borrow_unrelated_or_invalid_targets() {
    let application: PublicApplication = serde_json::from_value(serde_json::json!({
        "id": "7", "name": "Invalid Targets",
        "deeplink_uri": "javascript:alert(1)",
        "third_party_skus": [{"distributor": "steam", "id": "../invalid"}],
        "linked_games": [{"id": "42"}]
    }))
    .unwrap();
    let unrelated: Game = serde_json::from_value(serde_json::json!({
        "id": "99", "name": "Other Game",
        "third_party_skus": [{"distributor": "steam", "id": "480"}]
    }))
    .unwrap();
    let applications = HashMap::from([(7, Arc::new(application))]);
    let games = HashMap::from([(99, Arc::new(unrelated))]);
    assert_eq!(
        GameActions::resolve(7, &games, &applications),
        GameActions::default()
    );
    assert_eq!(
        GameActions::resolve(8, &games, &applications),
        GameActions::default()
    );
    assert_eq!(
        GameActions::resolve(99, &games, &applications),
        GameActions {
            steam_url: Some("https://store.steampowered.com/app/480".into()),
            play_url: None,
        }
    );
}

#[test]
fn game_launch_urls_only_accept_supported_launch_protocols() {
    for url in [
        "https://games.example/play",
        "steam://run/480",
        "steam://rungameid/480",
        "com.epicgames.launcher://apps/example?action=launch",
    ] {
        assert!(game_launch_url(url).is_some(), "{url}");
    }
    for url in [
        "",
        "not a URL",
        "javascript:alert(1)",
        "file:///tmp/game",
        "data:text/html,game",
        "https://user:password@games.example/play",
        "steam://uninstall/480",
        "com.epicgames.launcher://apps/example?action=uninstall",
    ] {
        assert!(game_launch_url(url).is_none(), "{url}");
    }
}

const COVER_SELECTORS: [&str; 5] = [
    "liked-game-cover-0",
    "liked-game-cover-1",
    "liked-game-cover-2",
    "liked-game-cover-3",
    "liked-game-cover-4",
];

struct CoverGrid {
    width: f32,
}

impl Render for CoverGrid {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().w(rpx(self.width)).child(
            game_cover_grid(COVER_SELECTORS.into_iter().map(|selector| {
                game_cover(None, ProfilePalette::of(None), cx)
                    .debug_selector(move || selector.into())
                    .into_any_element()
            }))
            .debug_selector(|| "liked-games-grid".into()),
        )
    }
}

#[gpui::test]
fn liked_game_covers_wrap_without_collapsing_the_collection(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, _| CoverGrid { width: 400. });
    for zoom in [1., 1.25, 1.5] {
        for (width, columns) in [(400., 4), (399., 3), (296., 3), (192., 2)] {
            view.update(cx, |grid, cx| {
                grid.width = width;
                cx.notify();
            });
            cx.update(|window, cx| {
                window.set_rem_size(px(theme::BASE_FONT_SIZE * zoom));
                window.refresh();
                let _ = window.draw(cx);
            });
            let grid = cx.debug_bounds("liked-games-grid").unwrap();
            for (index, selector) in COVER_SELECTORS.into_iter().enumerate() {
                let cover = cx.debug_bounds(selector).unwrap();
                let left = (index % columns) as f32 * (COVER_WIDTH + COVER_GAP) * zoom;
                let top = (index / columns) as f32 * (COVER_HEIGHT + COVER_GAP) * zoom;
                assert!((cover.left() - grid.left() - px(left)).abs() <= px(1.));
                assert!((cover.top() - grid.top() - px(top)).abs() <= px(1.));
                assert!((cover.size.width - px(COVER_WIDTH * zoom)).abs() <= px(1.));
                assert!((cover.size.height - px(COVER_HEIGHT * zoom)).abs() <= px(1.));
                assert!(cover.right() <= grid.right() + px(1.));
                assert!(cover.bottom() <= grid.bottom() + px(1.));
            }
        }
    }
}

fn game_entry(id: u64, name: &str, application_id: Option<u64>) -> OutboxEntry {
    OutboxEntry {
        id,
        started_at: None,
        ended_at: None,
        expires_at: None,
        kind: OutboxKind::PlayedGame {
            name: name.to_owned(),
            application_id,
        },
        traits: Vec::new(),
    }
}

#[test]
fn game_covers_match_the_compact_three_by_four_reference() {
    assert!((COVER_WIDTH - 87.61).abs() < 1.);
    assert!((COVER_HEIGHT - 116.81).abs() < 1.);
    assert!((COVER_WIDTH / COVER_HEIGHT - 3. / 4.).abs() < f32::EPSILON);
}

#[test]
fn current_activity_reuses_the_matching_history_badges() {
    let game = GameActivity {
        name: "Calculate It".into(),
        application_id: Some(7),
        platform: None,
        embedded: false,
        playable: false,
        kind: 0,
        started_at: None,
        ends_at: None,
        details: None,
        state: None,
        large_image: None,
        small_image: None,
    };
    let mut entry = game_entry(1, &game.name, game.application_id);
    entry.traits = vec![
        OutboxTrait::Live,
        OutboxTrait::FirstTime,
        OutboxTrait::Streak { days: 6 },
    ];
    let entries = [entry];
    let now = parse("2026-10-25T12:00:00Z").unwrap();
    let matched = current_entry(&game, &entries, now).unwrap();
    assert_eq!(matched.id, 1);
    assert_eq!(
        activity_traits(matched),
        [("sprout", "New Player".into()), ("zap", "6x Streak".into())]
    );
    assert!(current_entry(&game, &[], now).is_none());
}

#[test]
fn current_activity_prefers_ids_and_only_falls_back_when_an_id_is_missing() {
    let game = GameActivity {
        name: "Calculate It".into(),
        application_id: Some(7),
        platform: None,
        embedded: false,
        playable: false,
        kind: 0,
        started_at: None,
        ends_at: None,
        details: None,
        state: None,
        large_image: None,
        small_image: None,
    };
    let entries = [
        game_entry(1, "Localized title", Some(7)),
        game_entry(2, &game.name, Some(8)),
        game_entry(3, "calculate it", None),
    ];
    let now = parse("2026-10-25T12:00:00Z").unwrap();
    assert_eq!(current_entry(&game, &entries, now).unwrap().id, 1);
    assert_eq!(current_entry(&game, &entries[1..], now).unwrap().id, 3);
    assert!(current_entry(&game, &entries[1..2], now).is_none());
}

#[test]
fn current_activity_prefers_a_live_session_until_it_expires() {
    let game = GameActivity {
        name: "Calculate It".into(),
        application_id: Some(7),
        platform: None,
        embedded: false,
        playable: false,
        kind: 0,
        started_at: None,
        ends_at: None,
        details: None,
        state: None,
        large_image: None,
        small_image: None,
    };
    let mut live = game_entry(1, &game.name, game.application_id);
    live.traits.push(OutboxTrait::Live);
    live.expires_at = Some("2026-10-25T12:01:00Z".into());
    let mut recent = game_entry(2, &game.name, game.application_id);
    recent.ended_at = Some("2026-10-25T11:00:00Z".into());
    let mut old = game_entry(3, &game.name, game.application_id);
    old.ended_at = Some("2026-10-24T11:00:00Z".into());
    let entries = [live, recent, old];
    assert_eq!(
        current_entry(&game, &entries, parse("2026-10-25T12:00:00Z").unwrap())
            .unwrap()
            .id,
        1
    );
    assert_eq!(
        current_entry(&game, &entries, parse("2026-10-25T12:02:00Z").unwrap())
            .unwrap()
            .id,
        2
    );
}

#[test]
fn activity_badges_do_not_invent_traits_or_show_short_marathons_and_streaks() {
    let mut entry = game_entry(1, "Game", Some(7));
    assert!(activity_traits(&entry).is_empty());
    entry.traits = vec![
        OutboxTrait::Live,
        OutboxTrait::Unknown,
        OutboxTrait::Duration { seconds: 60 },
        OutboxTrait::Streak { days: 1 },
    ];
    assert!(activity_traits(&entry).is_empty());
    entry.traits = vec![
        OutboxTrait::Duration { seconds: 3 * 3600 },
        OutboxTrait::Trending,
    ];
    assert_eq!(
        activity_traits(&entry),
        [
            ("timer", "3h Marathon".into()),
            ("trending-up", "Trending".into())
        ]
    );
}

#[test]
fn clocks_read_like_discord() {
    assert_eq!(clock(Duration::from_secs(370)), "6:10");
    assert_eq!(clock(Duration::from_secs(18 * 60)), "18:00");
    assert_eq!(clock(Duration::from_secs(3723)), "1:02:03");
}

#[test]
fn tags_and_stats_read_like_discord() {
    assert_eq!(tag_label("better_than_you"), "Better than you");
    assert_eq!(tag_label("obsessed"), "Obsessed");
    assert_eq!(tag_label("lfg"), "LFG");
    assert_eq!(stat_name("achievement_num"), "Achievement");
}

#[test]
fn spans_round_like_discord() {
    let span_of = |minutes| span(chrono::TimeDelta::minutes(minutes));
    assert_eq!(span_of(0), "1m");
    assert_eq!(span_of(44), "44m");
    assert_eq!(span_of(50), "1h");
    assert_eq!(span_of(75), "1h");
    assert_eq!(span_of(60 * 23), "1d");
    assert_eq!(span_of(60 * 24 * 3), "3d");
    assert_eq!(span_of(60 * 24 * 11), "2w");
    assert_eq!(span_of(60 * 24 * 40), "1 month");
    assert_eq!(span_of(60 * 24 * 800), "2 years");
}

#[test]
fn wishlist_prices_and_launched_activity_times_read_like_discord() {
    let price = |amount, currency: &str, exponent| {
        sku_price(&SkuPrice {
            amount,
            currency: currency.to_owned(),
            exponent,
        })
    };
    assert_eq!(price(1399, "usd", 2), "$13.99");
    assert_eq!(price(2049, "eur", 2), "€20.49");
    assert_eq!(price(150000, "idr", 0), "150000 IDR");
    // A synthetic launched-game entry: no timestamps, only its snowflake id.
    assert_eq!(
        snowflake_time(1191169321074688097).unwrap().to_rfc3339(),
        "2024-01-01T00:01:37+00:00"
    );
}
