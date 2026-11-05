use super::serde_helpers::*;
use serde::Deserialize;

#[derive(Clone, Default, Deserialize)]
pub struct User {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub discriminator: Option<String>,
    #[serde(default)]
    pub bot: bool,
    #[serde(default)]
    pub premium_type: Option<u8>,
    #[serde(default, deserialize_with = "nullable")]
    pub primary_guild: Option<Option<PrimaryGuild>>,
    #[serde(default, deserialize_with = "nullable")]
    pub avatar_decoration_data: Option<Option<AvatarDecoration>>,
    #[serde(default, deserialize_with = "nullable")]
    pub collectibles: Option<Option<Collectibles>>,
    #[serde(default, deserialize_with = "nullable")]
    pub display_name_styles: Option<Option<DisplayNameStyles>>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub mfa_enabled: bool,
    #[serde(default)]
    pub verified: bool,
}

#[derive(Clone, Deserialize)]
pub struct UserUpdate {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default)]
    pub username: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub global_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub avatar: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub discriminator: Option<Option<String>>,
    #[serde(default)]
    pub bot: Option<bool>,
    #[serde(default, deserialize_with = "nullable")]
    pub primary_guild: Option<Option<PrimaryGuild>>,
    #[serde(default, deserialize_with = "nullable")]
    pub avatar_decoration_data: Option<Option<AvatarDecoration>>,
    #[serde(default, deserialize_with = "nullable")]
    pub collectibles: Option<Option<Collectibles>>,
    #[serde(default, deserialize_with = "nullable")]
    pub display_name_styles: Option<Option<DisplayNameStyles>>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub email: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub phone: Option<Option<String>>,
    #[serde(default)]
    pub mfa_enabled: Option<bool>,
}

#[derive(Clone, Deserialize)]
pub struct AvatarDecoration {
    pub asset: String,
    #[serde(default, deserialize_with = "optional_id")]
    pub sku_id: Option<u64>,
}

#[derive(Clone, Deserialize)]
pub struct Collectibles {
    #[serde(default, deserialize_with = "default_value")]
    pub nameplate: Option<Nameplate>,
}

#[derive(Clone, Deserialize)]
pub struct Nameplate {
    pub asset: String,
    #[serde(default, deserialize_with = "default_value")]
    pub palette: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub label: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    pub sku_id: Option<u64>,
}

#[derive(Clone, Deserialize)]
pub struct DisplayNameStyles {
    #[serde(default, deserialize_with = "default_value")]
    pub font_id: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub effect_id: Option<u32>,
    #[serde(default, deserialize_with = "entries")]
    pub colors: Vec<u32>,
}

#[derive(Clone, Deserialize)]
pub struct PrimaryGuild {
    #[serde(default)]
    pub identity_enabled: Option<bool>,
    #[serde(default, deserialize_with = "optional_id")]
    pub identity_guild_id: Option<u64>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub badge: Option<String>,
}
