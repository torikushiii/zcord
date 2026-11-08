use super::*;

#[test]
fn catalog_request_includes_bundles_variants_and_all_categories() {
    let request = catalog_request(&Client::new()).build().unwrap();
    assert_eq!(request.method(), reqwest::Method::GET);
    assert_eq!(
        request.url().as_str(),
        "https://discord.com/api/v9/collectibles-categories/v2?include_bundles=true&variants_return_style=2&skip_num_categories=0"
    );
    assert_eq!(
        request.headers()[REFERER],
        "https://discord.com/channels/@me"
    );
    assert!(request.body().is_none());
}

#[test]
fn catalog_flattens_bundles_and_deduplicates_in_category_product_item_order() {
    let bytes = serde_json::to_vec(&json!({
        "categories": [
            {"sku_id": "100", "name": "First category", "products": [
                {"sku_id": "900", "type": 3, "name": "First frame", "premium_type": 0, "items": [{"sku_id": "9", "type": 3}]},
                {"sku_id": "901", "type": 3000, "name": "Bundle", "premium_type": 2, "items": [
                    {"sku_id": "3", "type": 0, "asset": "decoration"},
                    {"sku_id": "1", "type": 1, "title": "Effect", "thumbnailPreviewSrc": "https://cdn.discordapp.com/preview.png", "effects": [{"src": "layer"}]},
                    {"sku_id": "2", "type": 2, "asset": "plate", "palette": "berry"},
                    {"sku_id": "4", "type": 3, "label": "Frame", "inner_width": 300, "layers": [{"id": "55"}]}
                ]},
                {"sku_id": "902", "type": 2000, "name": "Other", "items": [{"sku_id": "90", "type": 9}, {"sku_id": "5", "type": 1}]},
                {"sku_id": "903", "type": 1000, "name": "Empty", "items": []}
            ]},
            {"sku_id": "101", "name": "Second category", "products": [
                {"sku_id": "904", "type": 1, "name": "Duplicate effect", "items": [{"sku_id": "1", "type": 1, "title": "Duplicate"}]},
                {"sku_id": "905", "type": 0, "name": "Decoration", "items": [{"sku_id": "6", "type": 0}]},
                {"sku_id": "906", "type": 1000, "name": "Last bundle", "items": [{"sku_id": "9", "type": 3}, {"sku_id": "6", "type": 0}, {"sku_id": "7", "type": 2}]}
            ]}
        ],
        "collections": []
    })).unwrap();
    let products = decode_catalog(&bytes).unwrap();
    assert_eq!(
        products
            .iter()
            .map(|product| product.sku_id)
            .collect::<Vec<_>>(),
        vec![9, 3, 1, 2, 4, 5, 6, 7]
    );
    for product in &products {
        assert_eq!(product.items.len(), 1);
        assert_eq!(product.sku_id, product.items[0].sku_id);
        assert_eq!(product.kind, u16::from(product.items[0].kind));
    }
    assert_eq!(products[0].name, "First frame");
    assert_eq!(products[0].premium_type, Some(0));
    for product in &products[1..5] {
        assert_eq!(product.name, "Bundle");
        assert_eq!(product.premium_type, Some(2));
    }
    assert_eq!(products[2].items[0].title.as_deref(), Some("Effect"));
    assert_eq!(
        products[2].items[0].thumbnail_preview_src.as_deref(),
        Some("https://cdn.discordapp.com/preview.png")
    );
    assert_eq!(products[3].items[0].palette.as_deref(), Some("berry"));
    assert_eq!(products[4].items[0].layers[0].id, 55);
    assert_eq!(products[5].premium_type, None);
    assert_eq!(products[7].name, "Last bundle");
}

#[test]
fn catalog_accepts_empty_categories_and_rejects_malformed_responses() {
    assert!(
        decode_catalog(br#"{"categories":[],"collections":[]}"#)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        decode_catalog(br#"{"message":"missing categories"}"#),
        Err(Error::Decode)
    ));
    assert!(matches!(decode_catalog(b"not json"), Err(Error::Decode)));
}

fn set_patch() -> model::ProfileStylePatch {
    model::ProfileStylePatch {
        avatar: Some(Some("data:image/png;base64,avatar".into())),
        avatar_description: Some("42, added yesterday".into()),
        banner: Some(Some("data:image/png;base64,banner".into())),
        decoration: Some(Some(10)),
        nameplate: Some(Some(20)),
        display_name_font_id: Some(Some(11)),
        display_name_effect_id: Some(Some(1)),
        display_name_colors: Some(Some(vec![0x112233])),
        theme_colors: Some(Some([1, 2])),
        accent_color: Some(Some(3)),
        collectibles_sku_ids: Some(vec![30, 40]),
        ..Default::default()
    }
}

fn reset_patch() -> model::ProfileStylePatch {
    model::ProfileStylePatch {
        avatar: Some(None),
        banner: Some(None),
        decoration: Some(None),
        nameplate: Some(None),
        display_name_font_id: Some(None),
        display_name_effect_id: Some(None),
        display_name_colors: Some(None),
        theme_colors: Some(None),
        accent_color: Some(None),
        collectibles_sku_ids: Some(vec![]),
        ..Default::default()
    }
}

fn assert_request(
    guild: Option<u64>,
    profile: bool,
    patch: &model::ProfileStylePatch,
    url: &str,
    expected: Value,
) {
    let body = if profile {
        profile_body(patch, guild.is_some())
    } else {
        user_body(patch, guild.is_some())
    }
    .unwrap();
    let request = style_request(&Client::new(), guild, profile, &body)
        .build()
        .unwrap();
    assert_eq!(request.method(), reqwest::Method::PATCH);
    assert_eq!(request.url().as_str(), url);
    assert_eq!(
        request.headers()[REFERER],
        if guild.is_some() {
            "https://discord.com/channels/7"
        } else {
            "https://discord.com/channels/@me"
        }
    );
    assert_eq!(
        request.headers()[reqwest::header::CONTENT_TYPE],
        "application/json"
    );
    let actual: Value =
        serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn main_patch_set_and_reset() {
    assert_request(
        None,
        false,
        &set_patch(),
        "https://discord.com/api/v9/users/@me",
        json!({
            "avatar": "data:image/png;base64,avatar", "avatar_decoration_sku_id": "10", "nameplate_sku_id": "20",
            "display_name_font_id": 11, "display_name_effect_id": 1, "display_name_colors": [0x112233]
        }),
    );
    assert_request(
        None,
        true,
        &set_patch(),
        "https://discord.com/api/v9/users/@me/profile",
        json!({
            "banner": "data:image/png;base64,banner", "theme_colors": [1, 2], "accent_color": 3, "collectibles_sku_ids": ["30", "40"]
        }),
    );
    assert_request(
        None,
        false,
        &reset_patch(),
        "https://discord.com/api/v9/users/@me",
        json!({
            "avatar": null, "avatar_decoration_sku_id": null, "nameplate_sku_id": null,
            "display_name_font_id": null, "display_name_effect_id": null, "display_name_colors": null
        }),
    );
    assert_request(
        None,
        true,
        &reset_patch(),
        "https://discord.com/api/v9/users/@me/profile",
        json!({
            "banner": null, "theme_colors": [null, null], "accent_color": null, "collectibles_sku_ids": []
        }),
    );
}

#[test]
fn guild_patch_set_and_reset() {
    assert_request(
        Some(7),
        false,
        &set_patch(),
        "https://discord.com/api/v9/guilds/7/members/@me",
        json!({
            "avatar": "data:image/png;base64,avatar", "avatar_description": "42, added yesterday",
            "avatar_decoration_sku_id": "10", "collectibles": {"nameplate": {"sku_id": "20"}},
            "display_name_font_id": 11, "display_name_effect_id": 1, "display_name_colors": [0x112233]
        }),
    );
    assert_request(
        Some(7),
        true,
        &set_patch(),
        "https://discord.com/api/v9/guilds/7/profile/@me",
        json!({
            "banner": "data:image/png;base64,banner", "theme_colors": [1, 2], "collectibles_sku_ids": ["30", "40"]
        }),
    );
    assert_request(
        Some(7),
        false,
        &reset_patch(),
        "https://discord.com/api/v9/guilds/7/members/@me",
        json!({
            "avatar": null, "avatar_decoration_sku_id": null, "collectibles": {"nameplate": null},
            "display_name_font_id": null, "display_name_effect_id": null, "display_name_colors": null
        }),
    );
    assert_request(
        Some(7),
        true,
        &reset_patch(),
        "https://discord.com/api/v9/guilds/7/profile/@me",
        json!({
            "banner": null, "theme_colors": [null, null], "collectibles_sku_ids": []
        }),
    );
}

#[test]
fn patch_only_includes_changed_fields_and_skips_empty_requests() {
    let empty = model::ProfileStylePatch::default();
    for guild in [false, true] {
        assert_eq!(user_body(&empty, guild), None);
        assert_eq!(profile_body(&empty, guild), None);
    }
    let patch = model::ProfileStylePatch {
        nameplate: Some(None),
        ..Default::default()
    };
    assert_eq!(
        user_body(&patch, false),
        Some(json!({"nameplate_sku_id": null}))
    );
    assert_eq!(
        user_body(&patch, true),
        Some(json!({"collectibles": {"nameplate": null}}))
    );
    assert_eq!(profile_body(&patch, false), None);
    let patch = model::ProfileStylePatch {
        display_name_font_id: Some(Some(14)),
        ..Default::default()
    };
    assert_eq!(
        user_body(&patch, true),
        Some(json!({"display_name_font_id": 14}))
    );
    let patch = model::ProfileStylePatch {
        collectibles_sku_ids: Some(vec![30, 40]),
        ..Default::default()
    };
    assert_eq!(user_body(&patch, false), None);
    assert_eq!(
        profile_body(&patch, false),
        Some(json!({"collectibles_sku_ids": ["30", "40"]}))
    );
}

#[test]
fn recent_avatar_id_is_main_only() {
    let patch = model::ProfileStylePatch {
        avatar_id: Some(42),
        ..Default::default()
    };
    assert_eq!(user_body(&patch, false), Some(json!({"avatar_id": "42"})));
    assert_eq!(user_body(&patch, true), None);
}

#[test]
fn names_and_pronouns_go_where_discord_takes_them() {
    let patch = model::ProfileStylePatch {
        name: Some("New Name".into()),
        pronouns: Some("they/them".into()),
        ..Default::default()
    };
    assert_eq!(
        user_body(&patch, false),
        Some(json!({"global_name": "New Name"}))
    );
    assert_eq!(user_body(&patch, true), Some(json!({"nick": "New Name"})));
    for guild in [false, true] {
        assert_eq!(
            profile_body(&patch, guild),
            Some(json!({"pronouns": "they/them"}))
        );
    }
    let cleared = model::ProfileStylePatch {
        name: Some(String::new()),
        ..Default::default()
    };
    assert_eq!(user_body(&cleared, true), Some(json!({"nick": null})));
}

#[test]
fn clan_request_wears_or_drops_a_server_tag() {
    let wear = clan_request(&Client::new(), Some(890)).build().unwrap();
    assert_eq!(wear.method(), reqwest::Method::PUT);
    assert_eq!(
        wear.url().as_str(),
        "https://discord.com/api/v9/users/@me/clan"
    );
    let body: Value = serde_json::from_slice(wear.body().unwrap().as_bytes().unwrap()).unwrap();
    assert_eq!(
        body,
        json!({"identity_guild_id": "890", "identity_enabled": true})
    );
    let drop = clan_request(&Client::new(), None).build().unwrap();
    let body: Value = serde_json::from_slice(drop.body().unwrap().as_bytes().unwrap()).unwrap();
    assert_eq!(
        body,
        json!({"identity_guild_id": null, "identity_enabled": false})
    );
}

#[test]
fn purchases_flatten_bundles_deduplicate_and_keep_newest_metadata() {
    let purchases = serde_json::from_value(json!([
        {"sku_id": "100", "name": "Old", "type": 0, "purchased_at": "2026-01-01T00:00:00Z", "items": [{"type": 0, "sku_id": "1", "asset": "old"}]},
        {"sku_id": "200", "name": "Bundle", "type": 3000, "premium_type": 2, "purchased_at": "2026-03-01T00:00:00Z", "expires_at": "2027-01-01T00:00:00Z", "items": [
            {"type": 0, "sku_id": "1", "asset": "new"},
            {"type": 1, "sku_id": "2", "title": "Effect", "thumbnailPreviewSrc": "https://cdn.discordapp.com/effect.png", "effects": [{"src": "layer"}]},
            {"type": 2, "sku_id": "3", "asset": "nameplate", "palette": "berry"},
            {"type": 3, "sku_id": "4", "label": "Frame", "inner_width": 300, "overflow_top": 10, "layers": [{"id": "55"}]},
            {"type": 9, "sku_id": "5"}
        ]},
        {"sku_id": "300", "name": "Other bundle", "type": 1000, "purchased_at": "2026-02-01T00:00:00Z", "items": [{"type": 1, "sku_id": "6"}]},
        {"sku_id": "400", "name": "Undated", "type": 2, "items": [{"type": 2, "sku_id": "7"}]}
    ])).unwrap();
    let items = flatten_purchases(purchases);
    assert_eq!(
        items.iter().map(|p| p.product.sku_id).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 6, 7]
    );
    for item in &items {
        assert_eq!(item.product.items.len(), 1);
        assert_eq!(item.product.sku_id, item.product.items[0].sku_id);
        assert_eq!(item.product.kind, u16::from(item.product.items[0].kind));
    }
    assert_eq!(items[0].product.name, "Bundle");
    assert_eq!(items[0].product.premium_type, Some(2));
    assert_eq!(items[0].product.items[0].asset.as_deref(), Some("new"));
    assert_eq!(items[0].expires_at.as_deref(), Some("2027-01-01T00:00:00Z"));
    assert_eq!(items[1].product.items[0].title.as_deref(), Some("Effect"));
    assert_eq!(
        items[1].product.items[0].thumbnail_preview_src.as_deref(),
        Some("https://cdn.discordapp.com/effect.png")
    );
    assert_eq!(items[2].product.items[0].palette.as_deref(), Some("berry"));
    assert_eq!(items[3].product.items[0].layers[0].id, 55);
}
