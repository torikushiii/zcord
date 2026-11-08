use super::{CollectibleProduct, serde_helpers::*};
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct CollectiblePurchase {
    #[serde(flatten)]
    pub product: CollectibleProduct,
    #[serde(default, deserialize_with = "default_value")]
    pub purchased_at: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub expires_at: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct RecentAvatar {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    pub storage_hash: String,
    #[serde(default, deserialize_with = "default_value")]
    pub description: String,
}

#[derive(Clone, Default)]
pub struct ProfileStylePatch {
    pub avatar: Option<Option<String>>,
    pub avatar_id: Option<u64>,
    pub avatar_description: Option<String>,
    pub banner: Option<Option<String>>,
    pub decoration: Option<Option<u64>>,
    pub nameplate: Option<Option<u64>>,
    pub display_name_font_id: Option<Option<u32>>,
    pub display_name_effect_id: Option<Option<u32>>,
    pub display_name_colors: Option<Option<Vec<u32>>>,
    pub theme_colors: Option<Option<[u32; 2]>>,
    pub accent_color: Option<Option<u32>>,
    pub collectibles_sku_ids: Option<Vec<u64>>,
    pub name: Option<String>,
    pub pronouns: Option<String>,
    pub tag_guild: Option<Option<u64>>,
}
