use super::widgets::steam_app_id;
use super::*;

#[derive(Clone, Debug, Deserialize)]
pub struct Game {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    pub name: String,
    #[serde(default, deserialize_with = "default_value")]
    pub icon_hash: Option<String>,
    #[serde(
        rename = "third_party_skus",
        default,
        deserialize_with = "steam_app_id"
    )]
    pub steam_app_id: Option<String>,
    #[serde(default, deserialize_with = "entries")]
    pub linked_applications: Vec<LinkedApplication>,
    #[serde(default, deserialize_with = "entries")]
    pub websites: Vec<GameWebsite>,
    #[serde(default, deserialize_with = "default_value")]
    pub media: GameMedia,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LinkedApplication {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GameWebsite {
    pub url: String,
    #[serde(default, deserialize_with = "default_value")]
    pub category: u64,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct GameMedia {
    #[serde(default, deserialize_with = "default_value")]
    pub cover_url: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub cover_hash: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PublicApplication {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    pub name: String,
    #[serde(default, deserialize_with = "default_value")]
    pub icon: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub cover_image: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub deeplink_uri: Option<String>,
    #[serde(
        rename = "third_party_skus",
        default,
        deserialize_with = "steam_app_id"
    )]
    pub steam_app_id: Option<String>,
    #[serde(default, deserialize_with = "entries")]
    pub linked_games: Vec<LinkedGame>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LinkedGame {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub application: Option<Game>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Wishlist {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, rename = "wishlist_items", deserialize_with = "entries")]
    pub items: Vec<WishlistItem>,
    #[serde(default, deserialize_with = "entries")]
    pub applications: Vec<PublicApplication>,
}

#[derive(Clone, Debug)]
pub struct WishlistItem {
    pub sku_id: u64,
    pub name: String,
    pub is_owned: bool,
    pub application_id: Option<u64>,
    pub thumbnail_asset_id: Option<u64>,
    pub card_image_asset_id: Option<u64>,
    pub card_background_asset_id: Option<u64>,
    pub price: Option<SkuPrice>,
    pub collectible: Option<WishlistCollectible>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WishlistCollectible {
    #[serde(default, rename = "type")]
    pub kind: u8,
    #[serde(default, deserialize_with = "default_value")]
    pub asset: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub label: Option<String>,
}

impl<'de> Deserialize<'de> for WishlistItem {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct RawSku {
            #[serde(default, deserialize_with = "default_value")]
            name: String,
            #[serde(default, deserialize_with = "optional_id")]
            application_id: Option<u64>,
            #[serde(default, deserialize_with = "optional_id")]
            thumbnail_asset_id: Option<u64>,
            #[serde(default, deserialize_with = "default_value")]
            tenant_metadata: Option<RawTenantMetadata>,
            #[serde(default, deserialize_with = "default_value")]
            price: Option<SkuPrice>,
        }
        #[derive(Deserialize)]
        struct RawTenantMetadata {
            #[serde(default, deserialize_with = "default_value")]
            social_layer: Option<RawSocialLayer>,
        }
        #[derive(Deserialize)]
        struct RawSocialLayer {
            #[serde(default, deserialize_with = "optional_id")]
            card_image_asset_id: Option<u64>,
            #[serde(default, deserialize_with = "optional_id")]
            card_background_image_asset_id: Option<u64>,
        }
        #[derive(Deserialize)]
        struct Raw {
            #[serde(deserialize_with = "snowflake")]
            sku_id: u64,
            #[serde(default, deserialize_with = "default_value")]
            sku_name: Option<String>,
            #[serde(default, deserialize_with = "default_value")]
            is_owned: bool,
            #[serde(default, deserialize_with = "default_value")]
            collectibles_item: Option<WishlistCollectible>,
            #[serde(default, deserialize_with = "default_value")]
            sku: Option<RawSku>,
        }
        let raw = Raw::deserialize(d)?;
        let sku = raw.sku;
        Ok(Self {
            sku_id: raw.sku_id,
            name: raw
                .sku_name
                .or_else(|| sku.as_ref().map(|sku| sku.name.clone()))
                .unwrap_or_default(),
            is_owned: raw.is_owned,
            application_id: sku.as_ref().and_then(|sku| sku.application_id),
            thumbnail_asset_id: sku.as_ref().and_then(|sku| sku.thumbnail_asset_id),
            card_image_asset_id: sku.as_ref().and_then(|sku| {
                sku.tenant_metadata
                    .as_ref()?
                    .social_layer
                    .as_ref()?
                    .card_image_asset_id
            }),
            card_background_asset_id: sku.as_ref().and_then(|sku| {
                sku.tenant_metadata
                    .as_ref()?
                    .social_layer
                    .as_ref()?
                    .card_background_image_asset_id
            }),
            price: sku.and_then(|sku| sku.price),
            collectible: raw.collectibles_item,
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SkuPrice {
    #[serde(default, deserialize_with = "default_value")]
    pub amount: u64,
    #[serde(default)]
    pub currency: String,
    #[serde(
        default,
        rename = "currency_exponent",
        deserialize_with = "default_value"
    )]
    pub exponent: u32,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ApplicationIdentities {
    #[serde(default, deserialize_with = "entries")]
    pub identities: Vec<ApplicationIdentity>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ApplicationIdentity {
    #[serde(deserialize_with = "snowflake")]
    pub application_id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub profile: Option<IdentityProfile>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct IdentityProfile {
    #[serde(default, deserialize_with = "default_value")]
    pub username: Option<String>,
    #[serde(default, rename = "data", deserialize_with = "identity_stats")]
    pub stats: Vec<IdentityStat>,
}

impl IdentityProfile {
    pub fn image(&self, key: &str) -> Option<(&str, Option<u32>, Option<u32>)> {
        self.stats.iter().find_map(|stat| {
            if stat.name != key {
                return None;
            }
            let StatValue::Image { url, width, height } = &stat.value else {
                return None;
            };
            Some((url.as_str(), *width, *height))
        })
    }
}

fn identity_stats<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<IdentityStat>, D::Error> {
    #[derive(Deserialize)]
    struct Data {
        #[serde(default, deserialize_with = "entries")]
        dynamic: Vec<IdentityStat>,
        #[serde(default)]
        primary: Value,
    }
    let Some(data) = Option::<Data>::deserialize(d)? else {
        return Ok(Vec::new());
    };
    let mut stats = data.dynamic;
    let Some(primary) = data.primary.as_object() else {
        return Ok(stats);
    };
    for (name, value) in primary {
        if stats.iter().any(|stat| &stat.name == name) {
            continue;
        }
        if let Some(value) = primary_stat_value(value) {
            stats.push(IdentityStat {
                name: name.clone(),
                value,
            });
        }
    }
    Ok(stats)
}

fn primary_stat_value(value: &Value) -> Option<StatValue> {
    Some(match value {
        Value::Number(number) => StatValue::Number(number.as_f64()?),
        Value::String(text) => StatValue::Text(text.clone()),
        Value::Bool(flag) => StatValue::Text(flag.to_string()),
        Value::Object(object) => {
            let url = object
                .get("proxy_url")
                .and_then(Value::as_str)
                .or_else(|| object.get("url").and_then(Value::as_str))?
                .to_owned();
            let dimension = |key: &str| {
                object
                    .get(key)
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
            };
            StatValue::Image {
                url,
                width: dimension("width"),
                height: dimension("height"),
            }
        }
        Value::Null | Value::Array(_) => return None,
    })
}

#[derive(Clone, Debug)]
pub struct IdentityStat {
    pub name: String,
    pub value: StatValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatValue {
    Text(String),
    Number(f64),
    Image {
        url: String,
        width: Option<u32>,
        height: Option<u32>,
    },
}

#[derive(Deserialize)]
struct RawIdentityStat {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "type")]
    kind: u8,
    #[serde(default)]
    value: Value,
}

impl From<RawIdentityStat> for IdentityStat {
    fn from(raw: RawIdentityStat) -> Self {
        let value = match raw.kind {
            2 => StatValue::Number(
                raw.value
                    .as_f64()
                    .or_else(|| raw.value.as_str().and_then(|value| value.parse().ok()))
                    .unwrap_or_default(),
            ),
            _ => match &raw.value {
                Value::String(text) => StatValue::Text(text.clone()),
                Value::Object(_) => primary_stat_value(&raw.value)
                    .unwrap_or_else(|| StatValue::Text(raw.value.to_string())),
                other => StatValue::Text(other.to_string()),
            },
        };
        Self {
            name: raw.name,
            value,
        }
    }
}

impl<'de> Deserialize<'de> for IdentityStat {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        RawIdentityStat::deserialize(d).map(Self::from)
    }
}
