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
    if let Some(name) = &patch.name {
        let name = Some(name).filter(|name| !name.is_empty());
        body.insert(
            if guild { "nick" } else { "global_name" }.into(),
            json!(name),
        );
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
    if let Some(pronouns) = &patch.pronouns {
        body.insert("pronouns".into(), json!(pronouns));
    }
    (!body.is_empty()).then_some(Value::Object(body))
}

fn clan_request(client: &Client, guild_id: Option<u64>) -> RequestBuilder {
    client
        .put("https://discord.com/api/v9/users/@me/clan")
        .header(REFERER, "https://discord.com/channels/@me")
        .json(&json!({
            "identity_guild_id": guild_id.map(|id| id.to_string()),
            "identity_enabled": guild_id.is_some(),
        }))
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
        if let Some(tag_guild) = patch.tag_guild {
            self.send(|| clan_request(&self.client, tag_guild)).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
