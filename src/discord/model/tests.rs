use super::*;
use serde_json::{Value, json};

// Synthetic identities and assets; large IDs retain snowflake parsing coverage.
const PROFILE: &str = include_str!("../fixtures/profile_modal.json");
const PRODUCTS: &str = include_str!("../fixtures/collectibles_products.json");

#[test]
fn profile_parses_decorations_and_profile_fields() {
    let profile: Profile = serde_json::from_str(PROFILE).unwrap();
    let user = &profile.user.user;
    assert_eq!(user.id, 1191168931004416004);
    let decoration = user.avatar_decoration_data.clone().flatten().unwrap();
    assert_eq!(decoration.asset, "a_f000000000000000000000000000000b");
    assert_eq!(decoration.sku_id, Some(1191169212022784071));
    let nameplate = |collectibles: Option<&Collectibles>| {
        collectibles
            .and_then(|c| c.nameplate.as_ref())
            .map(|n| n.asset.clone())
    };
    assert_eq!(
        nameplate(user.collectibles.as_ref().and_then(Option::as_ref)).as_deref(),
        Some("nameplates/sample_plate_one/1191169216217088072/")
    );
    let member = profile.guild_member.as_ref().unwrap();
    assert_eq!(
        nameplate(member.member.collectibles.as_ref().and_then(Option::as_ref)).as_deref(),
        Some("nameplates/sample_plate_two/1191169149108224056/")
    );
    assert_eq!(
        member.member.roles.as_deref(),
        Some([1191168985530368017].as_slice())
    );
    let styles = user.display_name_styles.clone().flatten().unwrap();
    assert_eq!((styles.font_id, styles.effect_id), (Some(14), Some(8)));
    assert_eq!(styles.colors.len(), 4);
    let user_profile = profile.user_profile.as_ref().unwrap();
    assert_eq!(user_profile.pronouns.as_deref(), Some("they/them"));
    assert_eq!(
        user_profile.theme_colors.as_deref(),
        Some(&[11687486, 14261667][..])
    );
    assert_eq!(
        user_profile.profile_effect.as_ref().map(|e| e.sku_id),
        Some(1191169220411392073)
    );
    assert_eq!(
        profile.user.banner.as_deref(),
        Some("f000000000000000000000000000002e")
    );
    assert_eq!(profile.badges.len(), 5);
    assert_eq!(profile.badges[0].icon, "f000000000000000000000000000001c");
    assert_eq!(profile.connected_accounts.len(), 3);
    assert_eq!(profile.connected_accounts[1].kind, "github");
    let mutuals = profile.mutual_guilds.as_ref().unwrap();
    assert_eq!(mutuals.len(), 3);
    assert_eq!(mutuals[1].nick.as_deref(), Some("sample.user"));
    assert_eq!(profile.mutual_friends_count, Some(7));
    assert_eq!(profile.legacy_username.as_deref(), Some("Sample#0042"));
    let guild_profile = profile.guild_member_profile.as_ref().unwrap();
    assert!(guild_profile.profile_effect.is_none() && guild_profile.theme_colors.is_none());
    let frames: Vec<_> = user_profile
        .collectibles
        .iter()
        .map(|c| (c.sku_id, c.kind))
        .collect();
    assert_eq!(frames, [(1191169220411392073, 1), (1191169224605696074, 3)]);
}

#[test]
fn profile_survives_malformed_entries_and_missing_mutuals() {
    let mut value: Value = serde_json::from_str(PROFILE).unwrap();
    value["badges"][1] = json!({"id": 5});
    value["user"]["avatar_decoration_data"] = json!({"asset": false});
    value["user_profile"]["theme_colors"] = json!("nope");
    let object = value.as_object_mut().unwrap();
    object.remove("mutual_guilds");
    object.remove("mutual_friends_count");
    let profile: Profile = serde_json::from_value(value).unwrap();
    assert_eq!(profile.badges.len(), 4);
    assert!(profile.user.user.avatar_decoration_data.is_none());
    assert!(profile.mutual_guilds.is_none() && profile.mutual_friends_count.is_none());
}

#[test]
fn profile_parses_mutual_friends() {
    let mut value: Value = serde_json::from_str(PROFILE).unwrap();
    value["mutual_friends"] = json!([
        {"id": "42", "username": "friend", "global_name": "Friend"}
    ]);
    let profile: Profile = serde_json::from_value(value).unwrap();
    let friends = profile.mutual_friends.unwrap();
    assert_eq!(friends.len(), 1);
    assert_eq!(friends[0].id, 42);
    assert_eq!(friends[0].username, "friend");
}

#[test]
fn profile_reads_wishlist_ids_from_settings_keys() {
    let profile: Profile = serde_json::from_value(json!({
        "user": {"id": "1191169170079744061", "username": "sample.collector"},
        "wishlist_settings": {
            "1191169174274048062": {
                "visibility": 1,
                "updated_at": "2024-01-10T00:00:00.000+00:00"
            },
            "not-a-snowflake": {"visibility": 1}
        }
    }))
    .unwrap();
    assert_eq!(profile.wishlist_ids, [1191169174274048062]);

    let profile: Profile = serde_json::from_value(json!({
        "user": {"id": "1", "username": "bare"}
    }))
    .unwrap();
    assert!(profile.wishlist_ids.is_empty());
}

#[test]
fn wishlist_decodes_items_and_skips_malformed_entries() {
    let wishlist: Wishlist = serde_json::from_value(json!({
        "id": "1191169174274048062",
        "user_id": "1191169170079744061",
        "wishlist_items": [
            {
                "sku_id": "1191169300103168092",
                "sku_name": "Sample Add-on Bundle",
                "is_owned": false,
                "sku_product_line": "monthly",
                "sku": {
                    "application_id": "1191169081999360040",
                    "name": "Sample Add-on Bundle",
                    "thumbnail_asset_id": "1191169312686080095",
                    "tenant_metadata": {
                        "social_layer": {"card_image_asset_id": "1191169316880384096"}
                    },
                    "price": {
                        "amount": 2049,
                        "currency": "usd",
                        "currency_exponent": 2
                    }
                }
            },
            {"sku_name": "Malformed", "is_owned": true}
        ],
        "applications": [{
            "id": "1191169040056320030",
            "name": "Example Game",
            "icon": "icon-hash",
            "cover_image": "f0000000000000000000000000000025"
        }]
    }))
    .unwrap();
    assert_eq!(wishlist.id, 1191169174274048062);
    assert_eq!(wishlist.items.len(), 1);
    let item = &wishlist.items[0];
    assert_eq!(item.sku_id, 1191169300103168092);
    assert_eq!(item.name, "Sample Add-on Bundle");
    assert!(!item.is_owned);
    assert_eq!(item.application_id, Some(1191169081999360040));
    assert_eq!(item.thumbnail_asset_id, Some(1191169312686080095));
    assert_eq!(item.card_image_asset_id, Some(1191169316880384096));
    assert!(item.collectible.is_none());
    let price = item.price.as_ref().unwrap();
    assert_eq!(
        (price.amount, price.currency.as_str(), price.exponent),
        (2049, "usd", 2)
    );
    assert_eq!(wishlist.applications.len(), 1);
    assert_eq!(
        wishlist.applications[0].cover_image.as_deref(),
        Some("f0000000000000000000000000000025")
    );
}

#[test]
fn wishlist_decodes_collectible_items() {
    let wishlist: Wishlist = serde_json::from_value(json!({
        "id": "1191169174274048062",
        "wishlist_items": [{
            "sku_id": "1191169077805056039",
            "sku_product_line": 7,
            "sku_name": "Sample Avatar Decoration",
            "is_owned": false,
            "collectibles_item": {
                "type": 0,
                "sku_id": "1191169077805056039",
                "asset": "a_f0000000000000000000000000000029",
                "label": "Sample green decoration"
            },
            "bundle_items": null,
            "sku": {
                "id": "1191169077805056039",
                "type": 2,
                "product_line": 7,
                "application_id": "1191169010696192023",
                "name": "Sample Avatar Decoration",
                "thumbnail_asset_id": null,
                "price": {
                    "amount": 599,
                    "currency": "usd",
                    "currency_exponent": 2
                }
            }
        }],
        "applications": []
    }))
    .unwrap();
    let item = &wishlist.items[0];
    let collectible = item.collectible.as_ref().unwrap();
    assert_eq!(collectible.kind, 0);
    assert_eq!(
        collectible.asset.as_deref(),
        Some("a_f0000000000000000000000000000029")
    );
    assert_eq!(
        collectible.label.as_deref(),
        Some("Sample green decoration")
    );
    let price = item.price.as_ref().unwrap();
    assert_eq!(
        (price.amount, price.currency.as_str(), price.exponent),
        (599, "usd", 2)
    );
}

#[test]
fn wishlist_collectible_preserves_non_decoration_types() {
    for kind in [1, 2] {
        let collectible: WishlistCollectible = serde_json::from_value(json!({
            "type": kind,
            "asset": "synthetic-asset"
        }))
        .unwrap();
        assert_eq!(collectible.kind, kind);
    }
}

#[test]
fn user_badges_parse_leniently_and_match_profile_badges() {
    let badges: UserBadges = serde_json::from_value(json!({
        "badges": [
            {
                "badge_id": 1,
                "name": "Nitro",
                "description": "unexpected profile-style field",
                "info_label": "Subscriber since 1/1/24",
                "rarity": 2,
                "simple_icon_raster_url": "https://cdn.discordapp.com/assets/content/nitro.png",
                "complex_icon_static_url": "https://cdn.discordapp.com/assets/content/nitro.svg",
                "current_tier": "platinum",
                "tiers": [
                    {
                        "key": "platinum",
                        "name": "Platinum",
                        "simple_icon_raster_url": "https://cdn.discordapp.com/assets/content/tier.png"
                    },
                    {"key": "gold"}
                ],
                "unexpected": {"nested": true}
            },
            {"badge_id": 13},
            {"badge_id": 99, "name": "Mystery", "unexpected": true}
        ]
    }))
    .unwrap();
    assert_eq!(badges.badges.len(), 3);
    assert_eq!(badges.badges[0].name, "Nitro");
    assert_eq!(badges.badges[1].name, "");
    assert!(badges.badges[1].info_label.is_none());
    assert!(badges.badges[1].tiers.is_empty());
    assert_eq!(badges.badges[0].tier().unwrap().key, "platinum");
    assert!(badges.badges[1].tier().is_none());

    let badge = |id: &str, description: &str, icon: &str| Badge {
        id: id.to_owned(),
        description: description.to_owned(),
        icon: icon.to_owned(),
        link: None,
    };
    assert_eq!(
        badges
            .detail_for(&badge("unknown", "nope", "tier"))
            .map(|detail| detail.badge_id),
        Some(1)
    );
    assert_eq!(
        badges
            .detail_for(&badge("premium_tenure_12_month_v2", "nope", "none"))
            .map(|detail| detail.badge_id),
        Some(1)
    );
    assert_eq!(
        badges
            .detail_for(&badge("unknown", "mystery", "none"))
            .map(|detail| detail.badge_id),
        Some(99)
    );
    assert!(
        badges
            .detail_for(&badge("unknown", "nope", "none"))
            .is_none()
    );
    assert_eq!(badge_id_for("guild_booster_lvl7"), Some(12));
}

#[test]
fn user_cosmetics_distinguish_absent_from_null() {
    let absent: User = serde_json::from_value(json!({"id": "1"})).unwrap();
    assert!(absent.avatar_decoration_data.is_none());
    assert!(absent.collectibles.is_none() && absent.display_name_styles.is_none());
    let cleared: User = serde_json::from_value(json!({
        "id": "1",
        "avatar_decoration_data": null,
        "collectibles": null,
        "display_name_styles": null
    }))
    .unwrap();
    assert!(matches!(cleared.avatar_decoration_data, Some(None)));
    assert!(matches!(cleared.collectibles, Some(None)));
    assert!(matches!(cleared.display_name_styles, Some(None)));
    let member: PartialMember = serde_json::from_value(json!({
        "user": {"id": "1"},
        "collectibles": {"nameplate": {"asset": "nameplates/a/1/", "sku_id": "1"}}
    }))
    .unwrap();
    let collectibles = member.collectibles.flatten().unwrap();
    assert_eq!(collectibles.nameplate.unwrap().asset, "nameplates/a/1/");
}

#[test]
fn collectible_products_parse_effects_and_frames() {
    let products: Vec<CollectibleProduct> = serde_json::from_str(PRODUCTS).unwrap();
    assert_eq!(
        products.iter().map(|p| p.kind).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    let decoration = &products[0].items[0];
    assert_eq!(
        decoration.asset.as_deref(),
        Some("a_f0000000000000000000000000000016")
    );
    let effect = &products[1].items[0];
    assert_eq!(effect.kind, 1);
    assert!(effect.reduced_motion_src.is_some() && effect.accessibility_label.is_some());
    let layers: Vec<_> = effect
        .effects
        .iter()
        .map(|e| (e.looped, e.duration, e.start, e.loop_delay, e.z_index))
        .collect();
    assert_eq!(
        layers,
        [(false, 4981, 0, 0, 110), (true, 4980, 7981, 3000, 100)]
    );
    let nameplate = &products[2].items[0];
    assert_eq!(nameplate.palette.as_deref(), Some("forest"));
    let frame = &products[3].items[0];
    assert_eq!(frame.layers.len(), 6);
    assert_eq!(
        (
            frame.inner_width,
            frame.overflow_top,
            frame.overflow_bottom,
            frame.overflow_horizontal
        ),
        (Some(1200), Some(304), Some(212), Some(56))
    );
    let top = &frame.layers[1];
    assert_eq!(
        (
            top.id,
            top.kind.as_str(),
            top.order.as_str(),
            top.anchor.as_str(),
            top.responsive
        ),
        (1191169279131648087, "staple", "front", "top", false)
    );
}

#[test]
fn reply_tristate_and_depth_one_and_partial_fields() {
    let base =
        json!({"id":"1","channel_id":"2","author":{"id":"3"},"timestamp":"2026-10-22T00:00:00Z"});
    let message: Message = serde_json::from_value(base.clone()).unwrap();
    assert!(message.referenced_message.is_none());
    assert_eq!(message.kind, 0);
    let mut value = base;
    value["referenced_message"] = Value::Null;
    let message: Message = serde_json::from_value(value.clone()).unwrap();
    assert!(matches!(message.referenced_message, Some(None)));
    value["referenced_message"] =
        json!({"id":"4","author":{"id":"5"},"referenced_message":{"id":null,"author":false}});
    let message: Message = serde_json::from_value(value).unwrap();
    assert_eq!(message.referenced_message.unwrap().unwrap().id, 4);
    let update: MessageUpdate = serde_json::from_value(json!({"id":"1","channel_id":"2"})).unwrap();
    assert!(
        update.kind.is_none() && update.mentions.is_none() && update.referenced_message.is_none()
    );
    let update: MessageUpdate = serde_json::from_value(json!({"id":"1","channel_id":"2","referenced_message":null,"edited_timestamp":null,"mention_roles":["8","bad"],"future":true})).unwrap();
    assert!(matches!(update.referenced_message, Some(None)));
    assert_eq!(update.edited_timestamp, Some(None));
    assert_eq!(update.mention_roles, Some(vec![8]));
    let member: PartialMember =
        serde_json::from_value(json!({"user":{"id":"3"},"nick":null})).unwrap();
    assert_eq!(member.nick, Some(None));
    assert!(member.roles.is_none());
}

#[test]
fn nonce_accepts_strings_integers_and_garbage() {
    let base =
        json!({"id":"1","channel_id":"2","author":{"id":"3"},"timestamp":"2026-10-22T00:00:00Z"});
    let parse = |extra: Value| {
        let mut value = base.clone();
        value["nonce"] = extra;
        serde_json::from_value::<Message>(value).unwrap().nonce
    };
    assert_eq!(
        parse(json!("1176000000000000000")),
        Some(1176000000000000000)
    );
    assert_eq!(
        parse(json!(1176000000000000000u64)),
        Some(1176000000000000000)
    );
    assert_eq!(parse(json!("not-a-snowflake")), None);
    assert_eq!(parse(json!(null)), None);
    assert_eq!(parse(json!({"nested": true})), None);
    assert_eq!(serde_json::from_value::<Message>(base).unwrap().nonce, None);
}

#[test]
fn message_sticker_items_parse_and_default() {
    let base =
        json!({"id":"1","channel_id":"2","author":{"id":"3"},"timestamp":"2026-10-22T00:00:00Z"});
    let message: Message = serde_json::from_value(base.clone()).unwrap();
    assert!(message.sticker_items.is_empty());

    let message: Message = serde_json::from_value(json!({
        "id": "1",
        "channel_id": "2",
        "author": {"id": "3"},
        "timestamp": "2026-10-22T00:00:00Z",
        "sticker_items": [
            {"id": "99", "name": "wave", "format_type": 2},
            {"name": "no-id", "format_type": 1},
            {"id": "not-a-snowflake", "name": "skipped"}
        ]
    }))
    .unwrap();
    assert_eq!(message.sticker_items.len(), 2);
    assert_eq!(
        (
            message.sticker_items[0].id,
            message.sticker_items[0].name.as_str(),
            message.sticker_items[0].format_type
        ),
        (99, "wave", 2)
    );
    assert_eq!(
        (
            message.sticker_items[1].id,
            message.sticker_items[1].name.as_str(),
            message.sticker_items[1].format_type
        ),
        (0, "no-id", 1)
    );

    let snapshot: SnapshotMessage = serde_json::from_value(json!({
        "sticker_items": [{"id": "7", "name": "forwarded", "format_type": 4}]
    }))
    .unwrap();
    assert_eq!(snapshot.sticker_items[0].name, "forwarded");
    assert_eq!(snapshot.sticker_items[0].format_type, 4);

    let update: MessageUpdate = serde_json::from_value(json!({
        "id": "1",
        "channel_id": "2",
        "sticker_items": [{"id": "8", "name": "edited", "format_type": 3}]
    }))
    .unwrap();
    assert_eq!(update.sticker_items.unwrap()[0].format_type, 3);
    let plain: MessageUpdate =
        serde_json::from_value(json!({"id": "1", "channel_id": "2"})).unwrap();
    assert!(plain.sticker_items.is_none());
}

#[test]
fn message_snapshots_are_lenient() {
    let message: Message = serde_json::from_value(json!({
        "id": "1",
        "channel_id": "2",
        "author": {"id": "3"},
        "timestamp": "2026-10-22T00:00:00Z",
        "message_reference": {"type": 1, "channel_id": "4", "guild_id": "5"},
        "message_snapshots": [
            {"message": {
                "content": "forwarded",
                "embeds": [{}],
                "attachments": [{"id": "6"}],
                "components": [],
                "flags": 0,
                "type": 0,
                "timestamp": "not a timestamp",
                "edited_timestamp": "2026-10-22T01:00:00Z"
            }},
            {"message": "malformed"}
        ]
    }))
    .unwrap();
    assert_eq!(message.message_reference.unwrap().kind, 1);
    assert_eq!(message.message_snapshots.len(), 1);
    let snapshot = &message.message_snapshots[0].message;
    assert_eq!(snapshot.content, "forwarded");
    assert_eq!(snapshot.attachments[0].id, 6);
    assert_eq!(snapshot.embeds.len(), 1);
    assert!(snapshot.timestamp.is_none());
    assert_eq!(
        snapshot.edited_timestamp.as_deref(),
        Some("2026-10-22T01:00:00Z")
    );
    let update: MessageUpdate = serde_json::from_value(json!({
        "id": "1",
        "channel_id": "2",
        "message_snapshots": [{"message": {"content": "edited"}}]
    }))
    .unwrap();
    assert_eq!(
        update.message_snapshots.unwrap()[0].message.content,
        "edited"
    );
}

#[test]
fn profile_widgets_decode_per_entry() {
    let profile: Profile = serde_json::from_value(json!({
        "user": {"id": "1", "username": "synthetic"},
        "widgets": [
            {
                "id": "10",
                "data": {
                    "type": "favorite_games",
                    "games": [{"game_id": "20", "comment": "fun", "tags": ["cozy"]}]
                }
            },
            {"id": "11", "data": {"type": "application", "application_id": "30"}},
            {"id": "12", "data": {"type": "application", "application_id": "not-an-id"}},
            {"id": "13", "data": {"type": "unrecognized_widget"}},
            {
                "id": "14",
                "data": {
                    "type": "played_games",
                    "games": [{"game_id": "21"}, {"game_id": "22"}]
                }
            },
            {"data": {"type": "unrecognized_widget"}}
        ]
    }))
    .unwrap();
    assert_eq!(profile.widgets.len(), 4);
    let WidgetData::FavoriteGames { games } = &profile.widgets[0].data else {
        panic!("wrong widget kind");
    };
    assert_eq!(games[0].game_id, 20);
    assert_eq!(games[0].comment.as_deref(), Some("fun"));
    assert_eq!(games[0].tags, ["cozy"]);
    assert!(matches!(
        profile.widgets[1].data,
        WidgetData::Application { application_id: 30 }
    ));
    assert!(matches!(profile.widgets[2].data, WidgetData::Unknown));
    let WidgetData::PlayedGames { games } = &profile.widgets[3].data else {
        panic!("Games I Like was not decoded");
    };
    assert_eq!(
        games.iter().map(|game| game.game_id).collect::<Vec<_>>(),
        [21, 22]
    );
}

#[test]
fn profile_personal_widget_decodes_sections() {
    let profile: Profile = serde_json::from_value(json!({
        "user": {"id": "1", "username": "synthetic"},
        "widgets": [{
            "id": "10",
            "data": {
                "type": "personal",
                "header": "アキバ",
                "sections": [
                    {
                        "type": "cover",
                        "title": "",
                        "subtitle": "",
                        "image": {
                            "file_id": "1",
                            "width": 2400,
                            "height": 888,
                            "is_animated": false
                        }
                    },
                    {
                        "type": "fields",
                        "fields": [{
                            "title": "",
                            "description": "",
                            "image": {
                                "file_id": "2",
                                "width": 256,
                                "height": 256,
                                "is_animated": false
                            }
                        }]
                    }
                ]
            }
        }]
    }))
    .unwrap();
    let WidgetData::Personal { header, sections } = &profile.widgets[0].data else {
        panic!("personal widget was not decoded");
    };
    assert_eq!(header.as_deref(), Some("アキバ"));
    assert_eq!(sections.len(), 2);
    let PersonalSection::Cover {
        title,
        subtitle,
        image,
    } = &sections[0]
    else {
        panic!("cover section was not decoded");
    };
    assert!(title.is_none());
    assert!(subtitle.is_none());
    let image = image.as_ref().expect("cover image");
    assert_eq!(image.file_id, 1);
    assert_eq!((image.width, image.height), (Some(2400), Some(888)));
    assert!(!image.is_animated);

    let PersonalSection::Fields { fields } = &sections[1] else {
        panic!("fields section was not decoded");
    };
    assert_eq!(fields.len(), 1);
    assert!(fields[0].title.is_none());
    assert!(fields[0].description.is_none());
    let image = fields[0].image.as_ref().expect("field image");
    assert_eq!(image.file_id, 2);
    assert_eq!((image.width, image.height), (Some(256), Some(256)));
}

#[test]
fn profile_personal_widget_drops_malformed_sections() {
    let profile: Profile = serde_json::from_value(json!({
        "user": {"id": "1", "username": "synthetic"},
        "widgets": [{
            "id": "10",
            "data": {
                "type": "personal",
                "header": "",
                "sections": [
                    {"type": "cover", "title": "Kept", "subtitle": null},
                    {"type": "fields", "fields": "nope"}
                ]
            }
        }]
    }))
    .unwrap();
    let WidgetData::Personal { header, sections } = &profile.widgets[0].data else {
        panic!("personal widget was not decoded");
    };
    assert!(header.is_none());
    assert_eq!(sections.len(), 1);
    let PersonalSection::Cover {
        title,
        subtitle,
        image,
    } = &sections[0]
    else {
        panic!("cover section was not decoded");
    };
    assert_eq!(title.as_deref(), Some("Kept"));
    assert!(subtitle.is_none());
    assert!(image.is_none());
}

#[test]
fn presence_activity_decodes_application_and_timestamps() {
    let presence: Presence = serde_json::from_value(json!({
        "user_id": "2",
        "activities": [
            {
                "type": 0,
                "name": "Synthetic Game",
                "application_id": "123",
                "timestamps": {"start": 1700000000000u64, "end": "1700000001000"}
            },
            {
                "type": 0,
                "name": "Broken Game",
                "application_id": "not-an-id",
                "timestamps": {"start": "not-a-number", "end": null}
            },
            {
                "type": 0,
                "name": "String Timestamps",
                "timestamps": "not-an-object"
            }
        ]
    }))
    .unwrap();
    let activities = presence.activities.unwrap();
    assert_eq!(activities.len(), 3);
    assert_eq!(activities[0].application_id, Some(123));
    let timestamps = activities[0].timestamps.as_ref().unwrap();
    assert_eq!(timestamps.start, Some(1_700_000_000_000));
    assert!(activities[1].application_id.is_none());
    let timestamps = activities[1].timestamps.as_ref().unwrap();
    assert!(timestamps.start.is_none());
    assert!(activities[2].timestamps.is_none());
}

#[test]
fn presence_activity_decodes_platform_and_flags_leniently() {
    let presence: Presence = serde_json::from_value(json!({
        "user_id": "2",
        "activities": [
            {"type": 0, "name": "Console", "platform": "xbox", "flags": 256},
            {"type": 0, "name": "Desktop"},
            {"type": 0, "name": "Null Flags", "platform": null, "flags": null},
            {"type": 0, "name": "Garbage Flags", "platform": 7, "flags": "not-a-number"}
        ]
    }))
    .unwrap();
    let activities = presence.activities.unwrap();
    assert_eq!(activities.len(), 4);
    assert_eq!(activities[0].platform.as_deref(), Some("xbox"));
    assert_eq!(activities[0].flags, 256);
    assert!(activities[1].platform.is_none());
    assert_eq!(activities[1].flags, 0);
    assert!(activities[2].platform.is_none());
    assert_eq!(activities[2].flags, 0);
    assert!(activities[3].platform.is_none());
    assert_eq!(activities[3].flags, 0);
}

#[test]
fn presence_activity_decodes_created_at_from_gateway_payload() {
    let presence: Presence = serde_json::from_value(json!({
        "user_id": "2",
        "activities": [{
            "type": 0,
            "session_id": "test-session",
            "platform": "xbox",
            "official_game_id": null,
            "name": "Example Idle Game",
            "id": "f00000000000000f",
            "flags": 288,
            "details": "Example Adventurer",
            "created_at": 1704067200000u64,
            "content_classification": {"loaded": true, "data": null},
            "assets": {
                "large_image": "mp:avatars/1191168989724672018/a_f0000000000000000000000000000026.gif??size=4096&keep_aspect_ratio=true&quality=lossless"
            },
            "application_id": "1191168989724672018"
        }]
    }))
    .unwrap();
    let activities = presence.activities.unwrap();
    let activity = &activities[0];
    assert_eq!(activity.created_at, Some(1_704_067_200_000));
    assert_eq!(activity.flags, 288);
    assert_eq!(activity.platform.as_deref(), Some("xbox"));
    assert_eq!(activity.application_id, Some(1191168989724672018));
    assert!(activity.timestamps.is_none());
}

#[test]
fn games_and_public_applications_decode_per_entry() {
    let games: Vec<Game> = parse_entries(json!([
        {
            "id": "1",
            "name": "Synthetic Game",
            "icon_hash": "hash",
            "third_party_skus": [
                {"distributor": "gop", "id": "ignored"},
                {"distributor": "steam", "id": "3513350"},
                {"distributor": "steam", "id": "999"}
            ],
            "media": {
                "cover_url": "https://cdn.example/cover.png"
            }
        },
        {"id": "not-an-id", "name": "dropped"},
        {"id": "2", "name": "Bare"}
    ]));
    assert_eq!(games.len(), 2);
    assert_eq!(games[0].name, "Synthetic Game");
    assert_eq!(games[0].steam_app_id.as_deref(), Some("3513350"));
    assert_eq!(
        games[0].media.cover_url.as_deref(),
        Some("https://cdn.example/cover.png")
    );
    assert!(games[1].icon_hash.is_none());
    assert!(games[1].steam_app_id.is_none());
    assert!(games[1].media.cover_url.is_none());

    let applications: Vec<PublicApplication> = parse_entries(json!([
        {"id": "3", "name": "Synthetic App", "icon": "icon", "cover_image": "cover"},
        {"id": "bad", "name": "dropped"}
    ]));
    assert_eq!(applications.len(), 1);
    assert_eq!(applications[0].id, 3);
    assert_eq!(applications[0].icon.as_deref(), Some("icon"));
}

#[test]
fn game_action_metadata_preserves_partner_links_and_store_targets() {
    let application: PublicApplication = serde_json::from_value(json!({
        "id": "7",
        "name": "Partner App",
        "deeplink_uri": "https://games.example/play",
        "third_party_skus": [{"distributor": "steam", "id": 480}],
        "linked_games": [
            {
                "id": "42",
                "application": {
                    "id": "42",
                    "name": "Linked Game",
                    "third_party_skus": [{"distributor": "steam", "id": "481"}]
                }
            },
            {"id": "43"},
            {"id": "invalid"}
        ]
    }))
    .unwrap();
    assert_eq!(
        application.deeplink_uri.as_deref(),
        Some("https://games.example/play")
    );
    assert_eq!(application.steam_app_id.as_deref(), Some("480"));
    assert_eq!(application.linked_games.len(), 2);
    assert_eq!(application.linked_games[0].id, 42);
    assert_eq!(
        application.linked_games[0]
            .application
            .as_ref()
            .unwrap()
            .steam_app_id
            .as_deref(),
        Some("481")
    );
    assert!(application.linked_games[1].application.is_none());

    let game: Game = serde_json::from_value(json!({
        "id": "42",
        "name": "Linked Game",
        "linked_applications": [{"id": "7"}, {"id": "invalid"}],
        "websites": [
            {"category": 13, "url": "https://store.steampowered.com/app/481/"},
            {"category": 1, "url": "https://games.example/en/"}
        ]
    }))
    .unwrap();
    assert_eq!(game.linked_applications.len(), 1);
    assert_eq!(game.linked_applications[0].id, 7);
    assert_eq!(game.websites.len(), 2);
    assert_eq!(game.websites[1].category, 1);
    assert_eq!(game.websites[1].url, "https://games.example/en/");

    let bare: PublicApplication = serde_json::from_value(json!({
        "id": "8", "name": "No Actions",
        "deeplink_uri": null, "linked_games": null, "third_party_skus": null
    }))
    .unwrap();
    assert!(bare.deeplink_uri.is_none());
    assert!(bare.steam_app_id.is_none());
    assert!(bare.linked_games.is_empty());
}

#[test]
fn application_identity_stats_flatten_dynamic() {
    let identities: ApplicationIdentities = serde_json::from_value(json!({
        "identities": [
            {
                "application_id": "7",
                "profile": {
                    "username": "synthetic",
                    "data": {"dynamic": [
                        {"type": 1, "name": "server", "value": "SEA"},
                        {"type": 2, "name": "level", "value": 56.0},
                        {"type": 2, "name": "broken", "value": "not-a-number"}
                    ]}
                }
            },
            {"application_id": "bad"}
        ]
    }))
    .unwrap();
    assert_eq!(identities.identities.len(), 1);
    let profile = identities.identities[0].profile.as_ref().unwrap();
    assert_eq!(profile.username.as_deref(), Some("synthetic"));
    assert_eq!(profile.stats.len(), 3);
    assert_eq!(profile.stats[0].name, "server");
    assert_eq!(profile.stats[0].value, StatValue::Text("SEA".into()));
    assert_eq!(profile.stats[1].value, StatValue::Number(56.0));
    assert_eq!(profile.stats[2].value, StatValue::Number(0.0));
}

#[test]
fn application_identity_stats_include_primary_values() {
    let identities: ApplicationIdentities = serde_json::from_value(json!({
        "identities": [{
            "application_id": "1191169081999360040",
            "profile": {
                "username": "sample.player",
                "data": {
                    "primary": {
                        "season": "Sample Season",
                        "playtime_hours": 0.78,
                        "featured_played_character_image": {
                            "id": "1191169325268992098",
                            "url": "https://example.com/raw.png",
                            "proxy_url": "https://images-ext-1.discordapp.net/external/hash/raw.png",
                            "width": 1920,
                            "height": 2200
                        }
                    }
                }
            }
        }]
    }))
    .unwrap();
    let profile = identities.identities[0].profile.as_ref().unwrap();
    let names: Vec<_> = profile
        .stats
        .iter()
        .map(|stat| stat.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "season",
            "playtime_hours",
            "featured_played_character_image"
        ]
    );
    let stat = |name: &str| profile.stats.iter().find(|stat| stat.name == name).unwrap();
    assert_eq!(
        stat("season").value,
        StatValue::Text("Sample Season".into())
    );
    assert_eq!(stat("playtime_hours").value, StatValue::Number(0.78));
    assert_eq!(
        stat("featured_played_character_image").value,
        StatValue::Image {
            url: "https://images-ext-1.discordapp.net/external/hash/raw.png".into(),
            width: Some(1920),
            height: Some(2200),
        }
    );
    assert_eq!(
        profile.image("featured_played_character_image"),
        Some((
            "https://images-ext-1.discordapp.net/external/hash/raw.png",
            Some(1920),
            Some(2200)
        ))
    );
}

#[test]
fn application_identity_dynamic_image_stats() {
    let identities: ApplicationIdentities = serde_json::from_value(json!({
        "identities": [{
            "application_id": "7",
            "profile": {
                "data": {
                    "dynamic": [
                        {"type": 1, "name": "username", "value": "synthetic"},
                        {"type": 2, "name": "level", "value": 56.0},
                        {
                            "type": 3,
                            "name": "avatar",
                            "value": {
                                "id": "1",
                                "url": "https://example.com/a.png?x=1",
                                "proxy_url": "https://images-ext-1.discordapp.net/external/abc/%3Fx%3D1/https/example.com/a.png",
                                "width": 128,
                                "height": 128,
                                "content_type": "image/png"
                            }
                        }
                    ]
                }
            }
        }]
    }))
    .unwrap();
    let profile = identities.identities[0].profile.as_ref().unwrap();
    assert_eq!(
        profile.image("avatar"),
        Some((
            "https://images-ext-1.discordapp.net/external/abc/%3Fx%3D1/https/example.com/a.png",
            Some(128),
            Some(128)
        ))
    );
    let stat = |name: &str| profile.stats.iter().find(|stat| stat.name == name).unwrap();
    assert_eq!(stat("username").value, StatValue::Text("synthetic".into()));
    assert_eq!(stat("level").value, StatValue::Number(56.0));
}

#[test]
fn application_identity_dynamic_stats_win_over_primary_duplicates() {
    let identities: ApplicationIdentities = serde_json::from_value(json!({
        "identities": [{
            "application_id": "7",
            "profile": {
                "data": {
                    "dynamic": [
                        {"type": 2, "name": "level", "value": 56.0},
                        {"type": 1, "name": "server", "value": "SEA"}
                    ],
                    "primary": {
                        "server": "Ignored",
                        "season": "Sample Season"
                    }
                }
            }
        }]
    }))
    .unwrap();
    let profile = identities.identities[0].profile.as_ref().unwrap();
    let names: Vec<_> = profile
        .stats
        .iter()
        .map(|stat| stat.name.as_str())
        .collect();
    assert_eq!(names, ["level", "server", "season"]);
    let server = profile
        .stats
        .iter()
        .find(|stat| stat.name == "server")
        .unwrap();
    assert_eq!(server.value, StatValue::Text("SEA".into()));
}

#[test]
fn outbox_decodes_played_games_listening_and_traits() {
    let outbox: Outbox = serde_json::from_value(json!({
        "entries": [
            {
                "id": "10",
                "started_at": "2026-01-01T00:00:00Z",
                "traits": [
                    {"type": 2, "duration_seconds": 90},
                    {"type": 8, "streak_count_days": 3},
                    {"type": 9, "trending": 1}
                ],
                "extra": {
                    "type": "played_game_extra",
                    "game_name": "Synthetic Game",
                    "application_id": "20"
                }
            },
            {
                "id": "11",
                "expires_at": "2026-01-01T00:10:00Z",
                "traits": [{"type": 3, "is_live": true}],
                "extra": {
                    "type": "listened_session_extra",
                    "last_update": "2026-01-01T00:05:00Z",
                    "entries": [
                        {"media": {
                            "title": "Track One",
                            "parent_title": "Album",
                            "image_url": "https://i.scdn.co/image/synthetic",
                            "artists": [{"name": "Artist A"}, {"name": "Artist B"}]
                        }},
                        {"media": {"title": "Track Two", "artists": []}}
                    ]
                }
            },
            {
                "id": "12",
                "traits": [
                    {"type": 1, "first_time": true},
                    {"type": 5, "resurrected_last_played": "2025-12-01T00:00:00Z"}
                ],
                "extra": {"type": "played_game_extra", "game_name": "Another Game"}
            },
            {"id": "13", "extra": {"type": "future_extra"}},
            {"id": "bad", "extra": {"type": "played_game_extra"}}
        ]
    }))
    .unwrap();
    assert_eq!(outbox.entries.len(), 4);
    assert_eq!(outbox.entries[0].id, 10);
    assert_eq!(
        outbox.entries[0].started_at.as_deref(),
        Some("2026-01-01T00:00:00Z")
    );
    assert_eq!(
        outbox.entries[0].traits,
        [
            OutboxTrait::Duration { seconds: 90 },
            OutboxTrait::Streak { days: 3 },
            OutboxTrait::Trending
        ]
    );
    assert!(matches!(
        &outbox.entries[0].kind,
        OutboxKind::PlayedGame { name, application_id: Some(20) } if name == "Synthetic Game"
    ));
    assert_eq!(outbox.entries[1].traits, [OutboxTrait::Live]);
    assert_eq!(
        outbox.entries[1].expires_at.as_deref(),
        Some("2026-01-01T00:10:00Z")
    );
    let OutboxKind::Listened {
        tracks,
        last_update,
    } = &outbox.entries[1].kind
    else {
        panic!("wrong outbox kind");
    };
    assert_eq!(last_update.as_deref(), Some("2026-01-01T00:05:00Z"));
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].title, "Track One");
    assert_eq!(tracks[0].artists, ["Artist A", "Artist B"]);
    assert_eq!(
        tracks[0].image_url.as_deref(),
        Some("https://i.scdn.co/image/synthetic")
    );
    assert!(tracks[1].artists.is_empty());
    assert_eq!(
        outbox.entries[2].traits,
        [
            OutboxTrait::FirstTime,
            OutboxTrait::Resurrected {
                last_played: "2025-12-01T00:00:00Z".into()
            }
        ]
    );
    assert!(matches!(
        &outbox.entries[2].kind,
        OutboxKind::PlayedGame {
            application_id: None,
            ..
        }
    ));
    assert!(matches!(outbox.entries[3].kind, OutboxKind::Unknown));
    assert!(outbox.entries[3].traits.is_empty());
}

#[test]
fn outbox_decodes_launched_activities() {
    let outbox: Outbox = serde_json::from_value(json!({
        "entries": [{
            "id": "1191169321074688097",
            "author_id": "1191169170079744061",
            "content_type": 8,
            "traits": [{"type": 2, "duration_seconds": 243}],
            "extra": {
                "type": "launched_activity_extra",
                "activity_name": "Example Puzzle",
                "application_id": "1191169035862016029"
            },
            "participants": ["1191169170079744061"]
        }]
    }))
    .unwrap();
    assert_eq!(outbox.entries.len(), 1);
    assert_eq!(outbox.entries[0].author_id, Some(1191169170079744061));
    assert_eq!(outbox.entries[0].content_type, 8);
    assert_eq!(outbox.entries[0].participants, [1191169170079744061]);
    assert_eq!(
        outbox.entries[0].raw["extra"]["activity_name"], "Example Puzzle",
        "the raw entry is kept verbatim for replies"
    );
    let OutboxKind::LaunchedActivity {
        name,
        application_id,
        last_update,
    } = &outbox.entries[0].kind
    else {
        panic!("wrong outbox kind");
    };
    assert_eq!(name, "Example Puzzle");
    assert_eq!(*application_id, Some(1191169035862016029));
    assert!(last_update.is_none());
    assert_eq!(
        outbox.entries[0].traits,
        [OutboxTrait::Duration { seconds: 243 }]
    );
}

#[test]
fn content_inbox_sorts_ranked_entries_and_keeps_scheduling_fields() {
    let inbox: ContentInbox = serde_json::from_value(json!({
        "request_id": "synthetic",
        "entries": [
            {"rank": 3, "content": {
                "id": "1191169321074688099", "author_id": "1191169170079744063",
                "content_type": 1,
                "traits": [{"type": 4, "range": 1}, {"type": 2, "duration_seconds": 52105}],
                "extra": {"type": "played_game_extra", "game_name": "Rank Three",
                          "application_id": "303"}
            }},
            {"rank": 1, "content": {
                "id": "1191169321074688097", "author_id": "1191169170079744061",
                "content_type": 1,
                "traits": [{"type": 6, "marathon": true}, {"type": 7}],
                "extra": {"type": "played_game_extra", "game_name": "Rank One"}
            }},
            {"rank": 2, "content": {
                "id": "1191169321074688098", "author_id": "1191169170079744062",
                "content_type": 2,
                "traits": [{"type": 10, "count": 5}, {"type": 11, "count": 2},
                           {"type": 12, "count": 7}],
                "extra": {"type": "future_extra"}
            }},
            {"rank": 4, "content": {"id": "bad"}}
        ],
        "unranked_game_entries": [{"id": "1191169321074688100", "content_type": 1}],
        "expired_at": "2026-01-01T00:00:00Z",
        "entries_hash": "synthetic",
        "refresh_stale_inbox_after_ms": 60000,
        "refresh_token": "synthetic",
        "wait_ms_until_next_fetch": 120000
    }))
    .unwrap();
    assert_eq!(
        inbox.entries.len(),
        3,
        "unranked and malformed entries are dropped"
    );
    assert_eq!(inbox.entries[0].id, 1191169321074688097);
    assert_eq!(inbox.entries[0].author_id, Some(1191169170079744061));
    assert_eq!(inbox.entries[0].content_type, 1);
    assert_eq!(
        inbox.entries[0].traits,
        [OutboxTrait::Marathon, OutboxTrait::NewRelease]
    );
    assert_eq!(inbox.entries[1].id, 1191169321074688098);
    assert_eq!(
        inbox.entries[1].traits,
        [
            OutboxTrait::TopItemCount { count: 5 },
            OutboxTrait::TopParentCount { count: 2 },
            OutboxTrait::AggregateCount { count: 7 }
        ]
    );
    assert_eq!(inbox.entries[2].id, 1191169321074688099);
    assert_eq!(
        inbox.entries[2].traits,
        [
            OutboxTrait::AggregateRange { range: 1 },
            OutboxTrait::Duration { seconds: 52105 }
        ]
    );
    assert_eq!(inbox.wait_ms_until_next_fetch, Some(120_000));
}

#[test]
fn content_inbox_defaults_when_fields_are_missing_or_null() {
    for payload in [
        json!({}),
        json!({
            "entries": null, "expired_at": null,
            "refresh_stale_inbox_after_ms": null, "wait_ms_until_next_fetch": null
        }),
    ] {
        let inbox: ContentInbox = serde_json::from_value(payload).unwrap();
        assert!(inbox.entries.is_empty());
        assert!(inbox.wait_ms_until_next_fetch.is_none());
    }
}

#[test]
fn widget_configs_sort_stats_and_resolve_assets() {
    let configs = parse_featured_widget_configs(json!({
        "configs": {
            "100": [
                {
                    "application_id": "100",
                    "status": "draft",
                    "surfaces": {
                        "widget_top": {
                            "layout": "widget_top_hero",
                            "components": {
                                "title": {"fields": {"text": {
                                    "value_type": "custom_string",
                                    "value": "Draft"
                                }}}
                            }
                        }
                    }
                },
                {
                    "application_id": "100",
                    "display_name": "Sample Widget",
                    "status": "published",
                    "surfaces": {
                        "widget_top": {
                            "layout": "widget_top_hero",
                            "components": {
                                "title": {"fields": {"text": {
                                    "value_type": "data",
                                    "value": "username"
                                }}},
                                "subtitle_2": {"fields": {"text": {
                                    "value_type": "data",
                                    "value": "server_name"
                                }}},
                                "subtitle_10": {"fields": {"text": {
                                    "value_type": "custom_string",
                                    "value": "Tenth"
                                }}},
                                "subtitle_1": {"fields": {
                                    "text": {
                                        "value_type": "custom_string",
                                        "value": "First"
                                    },
                                    "label": {
                                        "value_type": "custom_string",
                                        "value": "label one"
                                    },
                                    "icon": {
                                        "value_type": "application_asset",
                                        "value": "icon"
                                    }
                                }},
                                "subtitle_3": {"fields": {"text": {
                                    "value_type": "custom_string",
                                    "value": "Third"
                                }}},
                                "subtitle_4": {"fields": {"text": {
                                    "value_type": "unsupported",
                                    "value": "Skipped"
                                }}},
                                "hero_image": {"fields": {"image": {
                                    "value_type": "application_asset",
                                    "value": "hero"
                                }}}
                            }
                        },
                        "widget_bottom": {
                            "layout": "widget_bottom_stats",
                            "components": {
                                "stat_10": {"fields": {
                                    "value": {"value_type": "data", "value": "level"},
                                    "label": {"value_type": "custom_string", "value": "Level"}
                                }},
                                "stat_2": {"fields": {
                                    "value": {"value_type": "custom_string", "value": "42"},
                                    "label": {"value_type": "custom_string", "value": "Two"}
                                }},
                                "stat_1": {"fields": {
                                    "value": {"value_type": "data", "value": "rank"},
                                    "label": {"value_type": "custom_string", "value": "Rank"}
                                }},
                                "stat_3": {"fields": {
                                    "value": {"value_type": "unsupported", "value": "x"},
                                    "label": {"value_type": "custom_string", "value": "Skipped"}
                                }}
                            }
                        }
                    },
                    "resolved_assets": [
                        {
                            "key": "hero",
                            "asset_id": "900",
                            "metadata": {"width": 640, "height": 480}
                        },
                        {
                            "key": "icon",
                            "asset_id": "902",
                            "metadata": {
                                "width": 32,
                                "height": 32,
                                "is_animated": true
                            }
                        },
                        {
                            "key": "other",
                            "asset_id": "901",
                            "metadata": {"width": 1, "height": 1}
                        }
                    ]
                }
            ]
        }
    }));
    assert_eq!(configs.len(), 1);
    let config = configs.get(&100).unwrap();
    assert_eq!(config.application_id, 100);
    assert_eq!(config.display_name.as_deref(), Some("Sample Widget"));
    assert_eq!(config.title, Some(WidgetText::Data("username".into())));
    assert_eq!(
        config.subtitles,
        [
            WidgetSubtitle {
                text: WidgetText::Literal("First".into()),
                label: Some(WidgetText::Literal("label one".into())),
                icon: Some(WidgetAsset {
                    asset_id: 902,
                    width: 32,
                    height: 32,
                    animated: true
                }),
            },
            WidgetSubtitle {
                text: WidgetText::Data("server_name".into()),
                label: None,
                icon: None,
            },
            WidgetSubtitle {
                text: WidgetText::Literal("Third".into()),
                label: None,
                icon: None,
            },
            WidgetSubtitle {
                text: WidgetText::Literal("Tenth".into()),
                label: None,
                icon: None,
            }
        ]
    );
    assert_eq!(
        config.hero,
        Some(WidgetImage::Asset(WidgetAsset {
            asset_id: 900,
            width: 640,
            height: 480,
            animated: false
        }))
    );
    assert!(config.collection.is_empty());
    assert_eq!(config.stats.len(), 3);
    assert_eq!(config.stats[0].value, WidgetText::Data("rank".into()));
    assert_eq!(config.stats[0].label, WidgetText::Literal("Rank".into()));
    assert_eq!(config.stats[1].value, WidgetText::Literal("42".into()));
    assert_eq!(config.stats[1].label, WidgetText::Literal("Two".into()));
    assert_eq!(config.stats[2].value, WidgetText::Data("level".into()));
}

#[test]
fn widget_collections_sort_items_and_skip_empty_ones() {
    let configs = parse_featured_widget_configs(json!({
        "configs": {
            "200": [{
                "application_id": "200",
                "display_name": "Collection",
                "surfaces": {
                    "widget_bottom": {
                        "layout": "widget_bottom_collection",
                        "components": {
                            "item_10": {"fields": {
                                "name": {"value_type": "custom_string", "value": "Tenth"}
                            }},
                            "item_2": {"fields": {
                                "name": {"value_type": "custom_string", "value": "Second"},
                                "description": {"value_type": "data", "value": "bio"},
                                "image": {"value_type": "application_asset", "value": "art"}
                            }},
                            "item_1": {"fields": {
                                "name": {"value_type": "custom_string", "value": "First"}
                            }},
                            "item_11": {"fields": {
                                "name": {"value_type": "unsupported", "value": "Skipped"},
                                "description": {"value_type": "unsupported", "value": "Skipped"},
                                "image": {"value_type": "unsupported", "value": "Skipped"}
                            }}
                        }
                    }
                },
                "resolved_assets": [{
                    "key": "art",
                    "asset_id": "300",
                    "metadata": {
                        "width": 120,
                        "height": 80,
                        "is_animated": true
                    }
                }]
            }]
        }
    }));
    let config = configs.get(&200).unwrap();
    assert_eq!(config.collection.len(), 3);
    assert_eq!(
        config.collection[0].name,
        Some(WidgetText::Literal("First".into()))
    );
    assert_eq!(
        config.collection[1].name,
        Some(WidgetText::Literal("Second".into()))
    );
    assert_eq!(
        config.collection[1].description,
        Some(WidgetText::Data("bio".into()))
    );
    assert_eq!(
        config.collection[1].image,
        Some(WidgetImage::Asset(WidgetAsset {
            asset_id: 300,
            width: 120,
            height: 80,
            animated: true
        }))
    );
    assert_eq!(
        config.collection[2].name,
        Some(WidgetText::Literal("Tenth".into()))
    );
    assert!(config.collection[2].description.is_none() && config.collection[2].image.is_none());
    assert!(config.stats.is_empty());
}

#[test]
fn widget_configs_parse_data_collection_images_and_contained_top_image() {
    let configs = parse_featured_widget_configs(json!({
        "configs": {
            "300": [{
                "application_id": "300",
                "surfaces": {
                    "widget_top": {
                        "layout": "widget_top_contained",
                        "components": {
                            "contained_image": {"fields": {"image": {
                                "value_type": "data",
                                "presentation_type": "image",
                                "value": "avatar"
                            }}},
                            "title": {"fields": {"text": {
                                "value_type": "data",
                                "value": "username"
                            }}}
                        }
                    },
                    "widget_bottom": {
                        "layout": "widget_bottom_collection",
                        "components": {
                            "item_1": {"fields": {
                                "description": {
                                    "value_type": "data",
                                    "presentation_type": "text",
                                    "value": "title_1_desc"
                                },
                                "image": {
                                    "value_type": "data",
                                    "presentation_type": "image",
                                    "value": "title_1_icon"
                                },
                                "name": {
                                    "value_type": "data",
                                    "presentation_type": "text",
                                    "value": "title_1_name"
                                }
                            }}
                        }
                    }
                }
            }]
        }
    }));
    let config = configs.get(&300).unwrap();
    assert_eq!(
        config.contained,
        Some(WidgetImage::Data {
            key: "avatar".into(),
            fallback: None
        })
    );
    assert_eq!(
        config.collection[0].image,
        Some(WidgetImage::Data {
            key: "title_1_icon".into(),
            fallback: None
        })
    );
}

#[test]
fn application_widget_configs_carry_the_popout_mini_profile() {
    // Synthetic response for `/applications/{id}/widget-configs`.
    let config = parse_application_widget_configs(
        json!([{
            "config_id": "1191169182662656064",
            "application_id": "1191169178468352063",
            "display_name": "Sample Widget",
            "status": "published",
            "surfaces": {
                "mini_profile": {"layout": "mini_profile_contained_stat", "components": {
                    "stat": {"fields": {"text": {"value_type": "custom_string", "presentation_type": "text", "value": "Sample statistic"}}},
                    "contained_image": {"fields": {"image": {"value_type": "application_asset", "presentation_type": "image", "value": "sample_contained_image"}}}
                }},
                "widget_top": {"layout": "widget_top_hero", "components": {
                    "title": {"fields": {"text": {"value_type": "custom_string", "presentation_type": "text", "value": "Sample heading"}}}
                }}
            },
            "resolved_assets": [{"key": "sample_contained_image", "asset_id": "1191169186856960065"}]
        }]),
        1191169178468352063,
    )
    .unwrap();
    assert_eq!(
        config.title,
        Some(WidgetText::Literal("Sample heading".into()))
    );
    let mini = config.mini.unwrap();
    assert_eq!(
        mini.text,
        Some(WidgetText::Literal("Sample statistic".into()))
    );
    assert!(
        matches!(mini.image, Some(WidgetImage::Asset(asset)) if asset.asset_id == 1191169186856960065)
    );
}

#[test]
fn widget_configs_carry_data_heroes() {
    let config = parse_application_widget_configs(
        json!([{
            "application_id": "1191168989724672018",
            "status": "published",
            "surfaces": {
                "widget_top": {
                    "layout": "widget_top_hero",
                    "components": {
                        "hero_image": {"fields": {"image": {
                            "value_type": "data",
                            "presentation_type": "image",
                            "value": "featured_played_character_image"
                        }}}
                    }
                }
            }
        }]),
        1191168989724672018,
    )
    .unwrap();
    assert_eq!(
        config.hero,
        Some(WidgetImage::Data {
            key: "featured_played_character_image".into(),
            fallback: None,
        })
    );
}

#[test]
fn widget_configs_carry_hero_fallbacks() {
    // Synthetic config with an application-asset fallback.
    let config = parse_application_widget_configs(
        json!([{
            "application_id": "1191169081999360040",
            "status": "published",
            "surfaces": {
                "widget_top": {
                    "layout": "widget_top_hero",
                    "components": {
                        "hero_image": {"fields": {"image": {
                            "value_type": "data",
                            "presentation_type": "image",
                            "value": "featured_played_character_image",
                            "fallback": {
                                "value_type": "application_asset",
                                "presentation_type": "image",
                                "value": "sample_hero"
                            }
                        }}}
                    }
                }
            },
            "resolved_assets": [{
                "key": "sample_hero",
                "asset_id": "1191169203634176069",
                "metadata": {"width": 2106, "height": 4681, "is_animated": false}
            }]
        }]),
        1191169081999360040,
    )
    .unwrap();
    assert_eq!(
        config.hero,
        Some(WidgetImage::Data {
            key: "featured_played_character_image".into(),
            fallback: Some(Box::new(WidgetImage::Asset(WidgetAsset {
                asset_id: 1191169203634176069,
                width: 2106,
                height: 4681,
                animated: false,
            }))),
        })
    );

    // A fallback that cannot be resolved is dropped, but the data hero survives.
    let config = parse_application_widget_configs(
        json!([{
            "application_id": "7",
            "status": "published",
            "surfaces": {
                "widget_top": {"components": {
                    "hero_image": {"fields": {"image": {
                        "value_type": "data",
                        "presentation_type": "image",
                        "value": "featured_played_character_image",
                        "fallback": {
                            "value_type": "application_asset",
                            "presentation_type": "image",
                            "value": "missing"
                        }
                    }}}
                }}
            }
        }]),
        7,
    )
    .unwrap();
    assert_eq!(
        config.hero,
        Some(WidgetImage::Data {
            key: "featured_played_character_image".into(),
            fallback: None,
        })
    );
}

#[test]
fn application_widget_configs_carry_the_activity_accessory() {
    // Synthetic response for `/applications/1191168989724672018/widget-configs`.
    let config = parse_application_widget_configs(
        json!([{
            "application_id": "1191168989724672018",
            "status": "published",
            "surfaces": {
                "activity_accessory": {
                    "layout": "activity_accessory_stat",
                    "components": {"stat": {"fields": {
                        "text": {
                            "value_type": "custom_string",
                            "presentation_type": "text",
                            "value": ""
                        },
                        "icon": {
                            "value_type": "application_asset",
                            "presentation_type": "image",
                            "value": "sample_accessory"
                        },
                        "label": {
                            "value_type": "custom_string",
                            "presentation_type": "text",
                            "value": "Current Mood"
                        }
                    }}}
                }
            },
            "resolved_assets": [{
                "key": "sample_accessory",
                "asset_id": "1191169207828480070",
                "metadata": {"width": 500, "height": 500, "is_animated": false}
            }]
        }]),
        1191168989724672018,
    )
    .unwrap();
    assert_eq!(
        config.accessory,
        Some(WidgetAccessory {
            layout: "activity_accessory_stat".into(),
            label: Some(WidgetText::Literal("Current Mood".into())),
            text: Some(WidgetText::Literal(String::new())),
            icon: Some(WidgetImage::Asset(WidgetAsset {
                asset_id: 1191169207828480070,
                width: 500,
                height: 500,
                animated: false,
            })),
        })
    );

    let config = parse_application_widget_configs(
        json!([{
            "application_id": "7",
            "status": "published",
            "surfaces": {"widget_top": {"components": {}}}
        }]),
        7,
    )
    .unwrap();
    assert!(config.accessory.is_none());
}

#[test]
fn widget_mini_profiles_parse_hero_and_contained_layouts() {
    let configs = parse_featured_widget_configs(json!({
        "configs": {
            "300": [{
                "application_id": "300",
                "status": "published",
                "surfaces": {
                    "mini_profile": {
                        "layout": "mini_profile_hero_stat",
                        "components": {
                            "stat": {"fields": {
                                "text": {
                                    "value_type": "data",
                                    "presentation_type": "text",
                                    "value": "rank_name"
                                },
                                "icon": {
                                    "value_type": "data",
                                    "presentation_type": "image",
                                    "value": "rank_image"
                                },
                                "label": {
                                    "value_type": "data",
                                    "presentation_type": "text",
                                    "value": "season"
                                }
                            }},
                            "hero_image": {"fields": {"image": {
                                "value_type": "data",
                                "presentation_type": "image",
                                "value": "featured_play"
                            }}}
                        }
                    }
                }
            }]
        }
    }));
    let mini = configs.get(&300).unwrap().mini.as_ref().unwrap();
    assert_eq!(mini.layout, "mini_profile_hero_stat");
    assert_eq!(mini.text, Some(WidgetText::Data("rank_name".into())));
    assert_eq!(mini.label, Some(WidgetText::Data("season".into())));
    assert_eq!(
        mini.icon,
        Some(WidgetImage::Data {
            key: "rank_image".into(),
            fallback: None,
        })
    );
    assert_eq!(
        mini.image,
        Some(WidgetImage::Data {
            key: "featured_play".into(),
            fallback: None,
        })
    );

    let configs = parse_featured_widget_configs(json!({
        "configs": {
            "301": [{
                "application_id": "301",
                "status": "published",
                "surfaces": {
                    "mini_profile": {
                        "layout": "mini_profile_contained_stat",
                        "components": {
                            "stat": {"fields": {
                                "text": {
                                    "value_type": "data",
                                    "presentation_type": "number",
                                    "value": "lev"
                                },
                                "icon": {
                                    "value_type": "application_asset",
                                    "presentation_type": "image",
                                    "value": "icon"
                                }
                            }},
                            "contained_image": {"fields": {"image": {
                                "value_type": "application_asset",
                                "presentation_type": "image",
                                "value": "art"
                            }}}
                        }
                    }
                },
                "resolved_assets": [
                    {
                        "key": "art",
                        "asset_id": "400",
                        "metadata": {"width": 64, "height": 64, "is_animated": false}
                    },
                    {"key": "icon", "asset_id": "401"}
                ]
            }]
        }
    }));
    let mini = configs.get(&301).unwrap().mini.as_ref().unwrap();
    assert_eq!(mini.layout, "mini_profile_contained_stat");
    assert_eq!(mini.text, Some(WidgetText::Data("lev".into())));
    assert_eq!(
        mini.icon,
        Some(WidgetImage::Asset(WidgetAsset {
            asset_id: 401,
            width: 0,
            height: 0,
            animated: false
        }))
    );
    assert_eq!(
        mini.image,
        Some(WidgetImage::Asset(WidgetAsset {
            asset_id: 400,
            width: 64,
            height: 64,
            animated: false
        }))
    );

    let configs = parse_featured_widget_configs(json!({
        "configs": {
            "302": [{
                "application_id": "302",
                "status": "published",
                "surfaces": {
                    "mini_profile": {
                        "layout": "mini_profile_contained_stat",
                        "components": {
                            "stat": {"fields": {"text": {
                                "value_type": "custom_string",
                                "value": "kept"
                            }}},
                            "contained_image": {"fields": {"image": {
                                "value_type": "application_asset",
                                "value": "missing"
                            }}}
                        }
                    }
                }
            }]
        }
    }));
    let mini = configs.get(&302).unwrap().mini.as_ref().unwrap();
    assert_eq!(mini.text, Some(WidgetText::Literal("kept".into())));
    assert!(mini.image.is_none());

    let configs = parse_featured_widget_configs(json!({
        "configs": {
            "303": [{
                "application_id": "303",
                "status": "published",
                "surfaces": {"widget_top": {"components": {}}}
            }]
        }
    }));
    assert!(configs.get(&303).unwrap().mini.is_none());
}

#[test]
fn application_widget_configs_accept_known_shapes() {
    let config = json!({
        "application_id": "7",
        "status": "published",
        "surfaces": {
            "widget_top": {
                "components": {
                    "title": {"fields": {"text": {
                        "value_type": "custom_string",
                        "value": "Title"
                    }}}
                }
            }
        }
    });
    for response in [
        json!([config.clone()]),
        json!({"configs": [config.clone()]}),
        json!({"configs": {"7": [config.clone()]}}),
        config.clone(),
    ] {
        let parsed = parse_application_widget_configs(response, 7).unwrap();
        assert_eq!(parsed.application_id, 7);
        assert_eq!(parsed.title, Some(WidgetText::Literal("Title".into())));
    }
    assert!(parse_application_widget_configs(json!({"configs": []}), 7).is_none());
    assert!(parse_application_widget_configs(json!({}), 7).is_none());
    assert!(parse_application_widget_configs(json!(null), 7).is_none());
}

#[test]
fn invite_links_extract_in_order_dedupe_and_cap() {
    let content = "join https://discord.gg/sampleInviteOne or https://www.discord.com/invite/sampleInviteTwo/ \
                   and (https://canary.discordapp.com/invite/abc_123?x=1) plus discord.gg/sampleInviteOne";
    assert_eq!(
        invite_codes(content),
        ["sampleInviteOne", "sampleInviteTwo", "abc_123"]
    );

    assert!(invite_codes(
        "https://notdiscord.gg/abc https://discord.gg.evil.com/abc https://evil.com/discord.gg/abc"
    )
    .is_empty());
    assert_eq!(
        invite_codes(
            "https://discord.gg/a1 https://discord.gg/a2 https://discord.gg/a3 https://discord.gg/a4"
        ),
        ["a1", "a2", "a3"]
    );
    assert_eq!(
        invite_codes("https://ptb.discord.com/invite/ptb1 https://discordapp.com/invite/dapp"),
        ["ptb1", "dapp"]
    );
}

#[test]
fn invite_decodes_profile_counts_and_games() {
    // Trimmed from GET /invites/sampleInviteOne?with_counts=true&with_expiration=true.
    let invite: Invite = serde_json::from_value(json!({
        "code": "sampleInviteOne",
        "expires_at": null,
        "guild": {
            "id": "1191168926810112003",
            "name": "Sample Server Two",
            "splash": null,
            "banner": null,
            "icon": "f000000000000000000000000000000a"
        },
        "guild_id": "1191168926810112003",
        "profile": {
            "id": "1191168926810112003",
            "name": "Sample Server Two",
            "icon_hash": "f000000000000000000000000000000a",
            "member_count": 87,
            "online_count": 25,
            "banner_hash": null,
            "custom_banner_hash": null,
            "game_application_ids": ["1191169119748096049", "1191169031667712028"]
        },
        "approximate_member_count": 87,
        "approximate_presence_count": 25
    }))
    .unwrap();
    assert_eq!(invite.code, "sampleInviteOne");
    assert_eq!(invite.guild_id, Some(1191168926810112003));
    assert_eq!(invite.guild.as_ref().unwrap().name, "Sample Server Two");
    let profile = invite.profile.as_ref().unwrap();
    assert_eq!(profile.member_count, Some(87));
    assert_eq!(profile.online_count, Some(25));
    assert_eq!(
        profile.game_application_ids,
        [1191169119748096049, 1191169031667712028]
    );

    let bare: Invite = serde_json::from_value(json!({
        "code": "bare",
        "guild_id": "1",
        "profile": {"member_count": null, "game_application_ids": "nope"}
    }))
    .unwrap();
    let profile = bare.profile.unwrap();
    assert!(profile.member_count.is_none());
    assert!(profile.game_application_ids.is_empty());
}

#[test]
#[ignore]
fn har_captures_decode() {
    let har = |dir: &str, prefix: &str| {
        std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .find(|entry| entry.file_name().to_string_lossy().starts_with(prefix))
            .and_then(|entry| std::fs::read_to_string(entry.path()).ok())
    };
    if let Some(text) = har("/tmp/har", "08_") {
        let profile: Profile = serde_json::from_str(&text).unwrap();
        assert!(!profile.widgets.is_empty());
    }
    if let Some(text) = har("/tmp/har", "16_") {
        let games: Vec<Game> = serde_json::from_str(&text).unwrap();
        assert!(!games.is_empty());
    }
    for prefix in ["13_", "21_"] {
        if let Some(text) = har("/tmp/har", prefix) {
            let outbox: Outbox = serde_json::from_str(&text).unwrap();
            assert!(!outbox.entries.is_empty());
        }
    }
    if let Some(text) = har("/tmp/har", "09_") {
        let identities: ApplicationIdentities = serde_json::from_str(&text).unwrap();
        assert!(!identities.identities.is_empty());
    }
    if let Some(text) = har("/tmp/har", "17_") {
        let applications: Vec<PublicApplication> = serde_json::from_str(&text).unwrap();
        assert!(!applications.is_empty());
    }
    if let Some(text) = har("/tmp/har2", "017_") {
        let configs = parse_featured_widget_configs(serde_json::from_str(&text).unwrap());
        assert!(!configs.is_empty());
        assert!(configs.values().any(|config| {
            config.stats.len() == 6
                && matches!(config.hero.as_ref(), Some(WidgetImage::Asset(asset))
                    if asset.asset_id != 0 && asset.width > 0 && asset.height > 0)
        }));
    }
    if let Some(text) = har("/tmp/har2", "018_") {
        let configs = parse_featured_widget_configs(serde_json::from_str(&text).unwrap());
        assert!(!configs.is_empty());
        assert!(configs.values().any(|config| {
            config
                .display_name
                .as_ref()
                .is_some_and(|name| !name.is_empty())
                && config.collection.len() == 4
                && config.subtitles.iter().any(|subtitle| {
                    subtitle.label.is_some()
                        && subtitle.icon.as_ref().is_some_and(|icon| icon.animated)
                })
        }));
    }
    if let Some(text) = har("/tmp/har2", "085_") {
        let games: Vec<Game> = serde_json::from_str(&text).unwrap();
        assert!(!games.is_empty());
        assert!(games.iter().any(|game| game.steam_app_id.is_some()));
        let raw: Vec<Value> = serde_json::from_str(&text).unwrap();
        assert_eq!(games.len(), raw.len());
        for (raw_game, game) in raw.iter().zip(&games) {
            let expected = raw_game
                .get("third_party_skus")
                .and_then(Value::as_array)
                .and_then(|skus| {
                    skus.iter()
                        .find(|sku| sku.get("distributor").and_then(Value::as_str) == Some("steam"))
                })
                .and_then(|sku| sku.get("id"))
                .and_then(|id| {
                    id.as_str()
                        .map(str::to_owned)
                        .or_else(|| id.as_u64().map(|id| id.to_string()))
                });
            assert_eq!(game.steam_app_id, expected);
        }
    }
}

#[test]
fn message_request_previews_keep_null_and_malformed_entries() {
    let previews: Vec<MessageRequestPreview> = serde_json::from_value(json!([
        {"channel_id": "1", "message_preview": null},
        {"channel_id": "2", "message_preview": {"id": "not-a-snowflake"}}
    ]))
    .unwrap();
    assert_eq!(previews.len(), 2);
    assert!(previews[0].message_preview.is_none());
    assert!(previews[1].message_preview.is_none());
}

#[test]
fn ready_and_voice_state_fill_voice_channel_members() {
    let mut payload: Value = serde_json::from_str(include_str!("../fixtures/ready.json")).unwrap();
    payload["guilds"][0]["voice_states"] = json!([
        {"channel_id": "2005", "user_id": "2", "self_mute": true, "self_deaf": false,
         "mute": false, "deaf": false, "self_stream": true, "self_video": false, "suppress": false},
        {"channel_id": null, "user_id": "1"}
    ]);
    let ready: Ready = serde_json::from_value(payload).unwrap();
    let states = &ready.guilds[0].voice_states;
    assert_eq!(states.len(), 2);
    assert_eq!(states[0].guild_id, Some(200));
    assert_eq!(states[0].channel_id, Some(2005));
    assert_eq!(states[0].user_id, 2);
    assert!(states[0].self_mute);
    assert!(states[0].self_stream);
    assert_eq!(states[1].channel_id, None);
    assert_eq!(states[1].guild_id, Some(200));

    let update: VoiceState = serde_json::from_value(json!({
        "guild_id": "200", "channel_id": "2005", "user_id": "3",
        "self_mute": false, "self_deaf": true, "mute": true, "deaf": false,
        "self_video": true, "suppress": true,
        "member": {"user": {"id": "3", "username": "speaker"}, "roles": ["201"]}
    }))
    .unwrap();
    assert_eq!(update.guild_id, Some(200));
    assert_eq!(update.channel_id, Some(2005));
    assert!(
        update
            .member
            .as_ref()
            .is_some_and(|member| member.id() == Some(3))
    );
}
