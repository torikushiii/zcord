use super::*;
use serde::Deserialize;
use serde_json::{Map, json};
use std::{cmp::Reverse, collections::HashSet};

fn user_body(patch: &model::ProfileStylePatch, guild: bool) -> Option<Value> {
    let mut body = Map::new();
    if let Some(avatar) = &patch.avatar {
        body.insert("avatar".into(), json!(avatar));
        if guild && let Some(description) = &patch.avatar_description {
            body.insert("avatar_description".into(), json!(description));
        }
    } else if !guild && let Some(id) = patch.avatar_id {
        body.insert("avatar_id".into(), json!(id.to_string()));
    }
    if let Some(sku) = patch.decoration {
        body.insert(
            "avatar_decoration_sku_id".into(),
            json!(sku.map(|id| id.to_string())),
        );
    }
    if let Some(sku) = patch.nameplate {
        if guild {
            body.insert(
                "collectibles".into(),
                json!({
                    "nameplate": sku.map(|id| json!({"sku_id": id.to_string()}))
                }),
            );
        } else {
            body.insert(
                "nameplate_sku_id".into(),
                json!(sku.map(|id| id.to_string())),
            );
        }
    }
    if let Some(font) = patch.display_name_font_id {
        body.insert("display_name_font_id".into(), json!(font));
    }
    if let Some(effect) = patch.display_name_effect_id {
        body.insert("display_name_effect_id".into(), json!(effect));
    }
    if let Some(colors) = &patch.display_name_colors {
        body.insert("display_name_colors".into(), json!(colors));
    }
    (!body.is_empty()).then_some(Value::Object(body))
}

fn profile_body(patch: &model::ProfileStylePatch, guild: bool) -> Option<Value> {
    let mut body = Map::new();
    if let Some(banner) = &patch.banner {
        body.insert("banner".into(), json!(banner));
    }
    if let Some(colors) = patch.theme_colors {
        body.insert(
            "theme_colors".into(),
            colors.map_or_else(|| json!([null, null]), |c| json!(c)),
        );
    }
    if !guild && let Some(accent) = patch.accent_color {
        body.insert("accent_color".into(), json!(accent));
    }
    if let Some(skus) = &patch.collectibles_sku_ids {
        body.insert(
            "collectibles_sku_ids".into(),
            json!(skus.iter().map(u64::to_string).collect::<Vec<_>>()),
        );
    }
    (!body.is_empty()).then_some(Value::Object(body))
}

fn style_request(
    client: &Client,
    guild_id: Option<u64>,
    profile: bool,
    body: &Value,
) -> RequestBuilder {
    let url = match (guild_id, profile) {
        (None, false) => "https://discord.com/api/v9/users/@me".to_owned(),
        (None, true) => "https://discord.com/api/v9/users/@me/profile".to_owned(),
        (Some(id), false) => format!("https://discord.com/api/v9/guilds/{id}/members/@me"),
        (Some(id), true) => format!("https://discord.com/api/v9/guilds/{id}/profile/@me"),
    };
    let referer = guild_id.map_or_else(
        || "https://discord.com/channels/@me".to_owned(),
        |id| format!("https://discord.com/channels/{id}"),
    );
    client.patch(url).header(REFERER, referer).json(body)
}

fn catalog_request(client: &Client) -> RequestBuilder {
    client
        .get("https://discord.com/api/v9/collectibles-categories/v2")
        .query(&[
            ("include_bundles", "true"),
            ("variants_return_style", "2"),
            ("skip_num_categories", "0"),
        ])
        .header(REFERER, "https://discord.com/channels/@me")
}

fn single_item_products(
    product: model::CollectibleProduct,
) -> impl Iterator<Item = model::CollectibleProduct> {
    product
        .items
        .into_iter()
        .filter(|item| item.kind <= 3)
        .map(move |item| model::CollectibleProduct {
            sku_id: item.sku_id,
            name: product.name.clone(),
            kind: u16::from(item.kind),
            premium_type: product.premium_type,
            items: vec![item],
        })
}

fn decode_catalog(bytes: &[u8]) -> Result<Vec<model::CollectibleProduct>, Error> {
    #[derive(Deserialize)]
    struct Category {
        #[serde(default, deserialize_with = "model::entries")]
        products: Vec<model::CollectibleProduct>,
    }
    #[derive(Deserialize)]
    struct Catalog {
        categories: Vec<Category>,
    }
    let catalog: Catalog = serde_json::from_slice(bytes).map_err(|_| Error::Decode)?;
    let mut seen = HashSet::new();
    Ok(catalog
        .categories
        .into_iter()
        .flat_map(|category| category.products)
        .flat_map(single_item_products)
        .filter(|product| seen.insert(product.sku_id))
        .collect())
}

fn flatten_purchases(
    mut purchases: Vec<model::CollectiblePurchase>,
) -> Vec<model::CollectiblePurchase> {
    purchases.sort_by_cached_key(|purchase| {
        Reverse(
            purchase
                .purchased_at
                .as_deref()
                .and_then(|date| chrono::DateTime::parse_from_rfc3339(date).ok()),
        )
    });
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for purchase in purchases {
        for product in single_item_products(purchase.product) {
            if !seen.insert(product.sku_id) {
                continue;
            }
            items.push(model::CollectiblePurchase {
                product,
                purchased_at: purchase.purchased_at.clone(),
                expires_at: purchase.expires_at.clone(),
            });
        }
    }
    items
}

impl Rest {
    pub async fn fetch_collectible_catalog(&self) -> Result<Vec<model::CollectibleProduct>, Error> {
        let response = self.send(|| catalog_request(&self.client)).await?;
        let bytes = response.bytes().await.map_err(|_| Error::Network)?;
        tokio::task::spawn_blocking(move || decode_catalog(&bytes))
            .await
            .map_err(|_| Error::Decode)?
    }

    pub async fn fetch_owned_collectibles(&self) -> Result<Vec<model::CollectiblePurchase>, Error> {
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/users/@me/collectibles-purchases")
                    .query(&[("variants_return_style", "2")])
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        let purchases = response.json().await.map_err(|_| Error::Decode)?;
        Ok(flatten_purchases(purchases))
    }

    pub async fn fetch_recent_avatars(&self) -> Result<Vec<model::RecentAvatar>, Error> {
        #[derive(Deserialize)]
        struct Avatars {
            avatars: Vec<model::RecentAvatar>,
        }
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/users/@me/avatars")
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        let mut avatars = response
            .json::<Avatars>()
            .await
            .map_err(|_| Error::Decode)?
            .avatars;
        avatars.truncate(6);
        Ok(avatars)
    }

    pub async fn recent_avatar_data_uri(&self, user_id: u64, hash: &str) -> Result<String, Error> {
        if hash.is_empty()
            || hash.len() > 128
            || !hash
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(Error::Decode);
        }
        let extension = if hash.starts_with("a_") { "gif" } else { "png" };
        let url =
            format!("https://cdn.discordapp.com/avatars/{user_id}/{hash}.{extension}?size=1024");
        // The CDN client deliberately has no account authorization headers.
        let mut response = self
            .upload_client
            .get(url)
            .send()
            .await
            .map_err(|_| Error::Network)?;
        if !response.status().is_success() {
            return Err(Error::Http(response.status().as_u16()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Network)? {
            if bytes.len().saturating_add(chunk.len()) >= 10 * 1024 * 1024 {
                return Err(Error::Decode);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(format!(
            "data:image/{extension};base64,{}",
            STANDARD.encode(bytes)
        ))
    }

    pub async fn patch_profile_styles(
        &self,
        guild_id: Option<u64>,
        patch: &model::ProfileStylePatch,
    ) -> Result<(), Error> {
        if let Some(body) = user_body(patch, guild_id.is_some()) {
            self.send(|| style_request(&self.client, guild_id, false, &body))
                .await?;
        }
        if let Some(body) = profile_body(patch, guild_id.is_some()) {
            self.send(|| style_request(&self.client, guild_id, true, &body))
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
}
