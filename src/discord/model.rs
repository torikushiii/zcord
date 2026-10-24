use serde::{Deserialize, Deserializer, de::DeserializeOwned};
use serde_json::Value;

pub fn snowflake<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let value = Value::deserialize(d)?;
    id_value(&value).ok_or_else(|| serde::de::Error::custom("invalid snowflake"))
}

pub fn id_value(value: &Value) -> Option<u64> {
    value
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| value.as_u64())
}

fn optional_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    Ok(id_value(&Value::deserialize(d)?))
}

fn optional_nonce<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(text) => text.parse().ok(),
        Value::Number(number) => number.as_u64(),
        _ => None,
    })
}

fn nullable_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<u64>>, D::Error> {
    optional_id(d).map(Some)
}

fn nullable_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

pub fn parse_entries<T: DeserializeOwned>(value: Value) -> Vec<T> {
    let Value::Array(entries) = value else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter_map(|entry| match serde_json::from_value(entry) {
            Ok(entry) => Some(entry),
            Err(_) => {
                log::warn!("Skipping malformed {} entry", std::any::type_name::<T>());
                None
            }
        })
        .collect()
}

fn entries<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    Value::deserialize(d).map(parse_entries)
}

fn optional_entries<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    d: D,
) -> Result<Option<Vec<T>>, D::Error> {
    Option::<Value>::deserialize(d).map(|v| v.map(parse_entries))
}

fn ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(id_value)
        .collect())
}

fn optional_ids<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u64>>, D::Error> {
    ids(d).map(Some)
}

fn default_value<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    d: D,
) -> Result<T, D::Error> {
    Ok(serde_json::from_value(Value::deserialize(d)?).unwrap_or_default())
}

fn nullable<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(if value.is_null() {
        Some(None)
    } else {
        serde_json::from_value(value).ok().map(Some)
    })
}

fn timestamp<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    chrono::DateTime::parse_from_rfc3339(&value)
        .map_err(|_| serde::de::Error::custom("invalid timestamp"))?;
    Ok(value)
}

fn optional_timestamp<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_str()
        .filter(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok())
        .map(str::to_owned))
}

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
pub struct Profile {
    pub user: ProfileUser,
    #[serde(default, deserialize_with = "default_value")]
    pub user_profile: Option<UserProfile>,
    #[serde(default, deserialize_with = "entries")]
    pub badges: Vec<Badge>,
    #[serde(default, deserialize_with = "entries")]
    pub connected_accounts: Vec<ConnectedAccount>,
    #[serde(default, deserialize_with = "default_value")]
    pub premium_type: Option<u8>,
    #[serde(default, deserialize_with = "default_value")]
    pub premium_since: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub premium_guild_since: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub legacy_username: Option<String>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub mutual_guilds: Option<Vec<MutualGuild>>,
    #[serde(default, deserialize_with = "default_value")]
    pub mutual_friends_count: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub guild_member: Option<ProfileMember>,
    #[serde(default, deserialize_with = "default_value")]
    pub guild_member_profile: Option<UserProfile>,
}

#[derive(Clone, Deserialize)]
pub struct ProfileUser {
    #[serde(flatten)]
    pub user: User,
    #[serde(default, deserialize_with = "default_value")]
    pub banner: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub banner_color: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub accent_color: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub bio: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct UserProfile {
    #[serde(default, deserialize_with = "default_value")]
    pub bio: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub pronouns: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub accent_color: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub banner: Option<String>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub theme_colors: Option<Vec<u32>>,
    #[serde(default, deserialize_with = "default_value")]
    pub profile_effect: Option<ProfileEffectRef>,
    #[serde(default, deserialize_with = "entries")]
    pub collectibles: Vec<ProfileCollectible>,
}

#[derive(Clone, Deserialize)]
pub struct ProfileCollectible {
    #[serde(deserialize_with = "snowflake")]
    pub sku_id: u64,
    #[serde(default, rename = "type", deserialize_with = "default_value")]
    pub kind: u8,
}

#[derive(Clone, Deserialize)]
pub struct ProfileEffectRef {
    #[serde(deserialize_with = "snowflake")]
    pub sku_id: u64,
}

#[derive(Clone, Deserialize)]
pub struct Badge {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default, deserialize_with = "default_value")]
    pub link: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct ConnectedAccount {
    #[serde(default, rename = "type", deserialize_with = "default_value")]
    pub kind: String,
    #[serde(default, deserialize_with = "default_value")]
    pub id: String,
    #[serde(default, deserialize_with = "default_value")]
    pub name: String,
    #[serde(default, deserialize_with = "default_value")]
    pub verified: bool,
}

#[derive(Clone, Deserialize)]
pub struct MutualGuild {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub nick: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct ProfileMember {
    #[serde(flatten)]
    pub member: PartialMember,
    #[serde(default, deserialize_with = "default_value")]
    pub banner: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub joined_at: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct CollectibleProduct {
    #[serde(deserialize_with = "snowflake")]
    pub sku_id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub name: String,
    #[serde(default, rename = "type", deserialize_with = "default_value")]
    pub kind: u8,
    #[serde(default, deserialize_with = "entries")]
    pub items: Vec<CollectibleItem>,
}

#[derive(Clone, Deserialize)]
pub struct CollectibleItem {
    #[serde(default, rename = "type", deserialize_with = "default_value")]
    pub kind: u8,
    #[serde(deserialize_with = "snowflake")]
    pub sku_id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub asset: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub label: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub palette: Option<String>,
    #[serde(
        default,
        rename = "accessibilityLabel",
        deserialize_with = "default_value"
    )]
    pub accessibility_label: Option<String>,
    #[serde(
        default,
        rename = "reducedMotionSrc",
        deserialize_with = "default_value"
    )]
    pub reduced_motion_src: Option<String>,
    #[serde(default, deserialize_with = "entries")]
    pub effects: Vec<EffectLayer>,
    #[serde(default, deserialize_with = "entries")]
    pub layers: Vec<FrameLayer>,
    #[serde(default, deserialize_with = "default_value")]
    pub inner_width: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub overflow_top: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub overflow_bottom: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub overflow_horizontal: Option<u32>,
}

#[derive(Clone, Deserialize)]
pub struct EffectLayer {
    #[serde(default, deserialize_with = "default_value")]
    pub src: String,
    #[serde(default, rename = "loop", deserialize_with = "default_value")]
    pub looped: bool,
    #[serde(default, deserialize_with = "default_value")]
    pub duration: u32,
    #[serde(default, deserialize_with = "default_value")]
    pub start: u32,
    #[serde(default, rename = "loopDelay", deserialize_with = "default_value")]
    pub loop_delay: u32,
    #[serde(default, rename = "zIndex", deserialize_with = "default_value")]
    pub z_index: i32,
}

#[derive(Clone, Deserialize)]
pub struct FrameLayer {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, rename = "type", deserialize_with = "default_value")]
    pub kind: String,
    #[serde(default, deserialize_with = "default_value")]
    pub order: String,
    #[serde(default, deserialize_with = "default_value")]
    pub anchor: String,
    #[serde(default, deserialize_with = "default_value")]
    pub responsive: bool,
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

#[derive(Clone, Deserialize)]
pub struct Member {
    #[serde(default, deserialize_with = "optional_id")]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default, deserialize_with = "ids")]
    pub roles: Vec<u64>,
    #[serde(default)]
    pub nick: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub avatar_decoration_data: Option<Option<AvatarDecoration>>,
    #[serde(default, deserialize_with = "nullable")]
    pub display_name_styles: Option<Option<DisplayNameStyles>>,
    #[serde(default, deserialize_with = "nullable")]
    pub collectibles: Option<Option<Collectibles>>,
}

impl Member {
    pub fn id(&self) -> Option<u64> {
        self.user_id.or_else(|| self.user.as_ref().map(|u| u.id))
    }
}

#[derive(Clone, Deserialize)]
pub struct Role {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "snowflake")]
    pub permissions: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub color: u32,
    #[serde(default, deserialize_with = "default_value")]
    pub colors: RoleColors,
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub hoist: bool,
}

#[derive(Clone, Default, Deserialize)]
pub struct RoleColors {
    #[serde(default)]
    pub primary_color: u32,
}

#[derive(Clone, Deserialize)]
pub struct Overwrite {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(deserialize_with = "snowflake")]
    pub allow: u64,
    #[serde(deserialize_with = "snowflake")]
    pub deny: u64,
}

#[derive(Clone, Deserialize)]
pub struct Channel {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default, rename = "type")]
    pub kind: Option<u8>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub position: Option<i64>,
    #[serde(default, deserialize_with = "nullable_id")]
    pub parent_id: Option<Option<u64>>,
    #[serde(default, deserialize_with = "nullable_id")]
    pub last_message_id: Option<Option<u64>>,
    #[serde(default)]
    pub permission_overwrites: Option<Vec<Overwrite>>,
    #[serde(default, deserialize_with = "ids")]
    pub recipient_ids: Vec<u64>,
    #[serde(default, deserialize_with = "entries")]
    pub recipients: Vec<User>,
}

impl Channel {
    pub fn merge(&mut self, update: &Self) {
        macro_rules! merge { ($($field:ident),*) => { $(if update.$field.is_some() { self.$field = update.$field.clone(); })* }; }
        merge!(
            guild_id,
            kind,
            name,
            position,
            parent_id,
            last_message_id,
            permission_overwrites
        );
        if !update.recipient_ids.is_empty() {
            self.recipient_ids.clone_from(&update.recipient_ids);
        }
        if !update.recipients.is_empty() {
            self.recipients.clone_from(&update.recipients);
        }
    }
}

#[derive(Clone, Default, Deserialize)]
pub struct GuildProperties {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    pub owner_id: Option<u64>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub icon: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub banner: Option<Option<String>>,
}

#[derive(Clone, Deserialize)]
pub struct Guild {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default)]
    pub properties: GuildProperties,
    #[serde(flatten)]
    pub legacy: GuildProperties,
    #[serde(default, deserialize_with = "optional_entries")]
    pub channels: Option<Vec<Channel>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub threads: Option<Vec<Channel>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub roles: Option<Vec<Role>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub members: Option<Vec<Member>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub emojis: Option<Vec<GuildEmoji>>,
    #[serde(default)]
    pub unavailable: Option<bool>,
}

impl Guild {
    pub fn name(&self) -> &str {
        self.properties
            .name
            .as_deref()
            .or(self.legacy.name.as_deref())
            .unwrap_or("")
    }
    pub fn owner_id(&self) -> Option<u64> {
        self.properties.owner_id.or(self.legacy.owner_id)
    }
    pub fn icon(&self) -> Option<String> {
        self.properties
            .icon
            .as_ref()
            .or(self.legacy.icon.as_ref())
            .cloned()
            .flatten()
    }
    pub fn banner(&self) -> Option<String> {
        self.properties
            .banner
            .as_ref()
            .or(self.legacy.banner.as_ref())
            .cloned()
            .flatten()
    }
    pub fn merge(&mut self, update: &Self) {
        if let Some(name) = update
            .properties
            .name
            .as_ref()
            .or(update.legacy.name.as_ref())
        {
            self.properties.name = Some(name.clone());
        }
        if let Some(id) = update.owner_id() {
            self.properties.owner_id = Some(id);
        }
        if let Some(icon) = update
            .properties
            .icon
            .as_ref()
            .or(update.legacy.icon.as_ref())
        {
            self.properties.icon = Some(icon.clone());
        }
        if let Some(banner) = update
            .properties
            .banner
            .as_ref()
            .or(update.legacy.banner.as_ref())
        {
            self.properties.banner = Some(banner.clone());
        }
        if update.channels.is_some() {
            self.channels.clone_from(&update.channels);
        }
        if update.threads.is_some() {
            self.threads.clone_from(&update.threads);
        }
        if update.roles.is_some() {
            self.roles.clone_from(&update.roles);
        }
        if update.members.is_some() {
            self.members.clone_from(&update.members);
        }
        if update.emojis.is_some() {
            self.emojis.clone_from(&update.emojis);
        }
        if update.unavailable.is_some() {
            self.unavailable = update.unavailable;
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct GuildEmoji {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub animated: bool,
    #[serde(default = "default_true")]
    pub available: bool,
    #[serde(default, deserialize_with = "ids")]
    pub roles: Vec<u64>,
    #[serde(default = "default_true")]
    pub require_colons: bool,
}

#[derive(Clone, Deserialize)]
pub struct GuildEmojisUpdate {
    #[serde(deserialize_with = "snowflake")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "entries")]
    pub emojis: Vec<GuildEmoji>,
}

#[derive(Clone, Deserialize)]
pub struct ReadState {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "optional_id")]
    pub last_message_id: Option<u64>,
    #[serde(default)]
    pub mention_count: u32,
    #[serde(default, rename = "read_state_type")]
    pub kind: u8,
}

#[derive(Clone, Default, Deserialize)]
pub struct ReadStates {
    #[serde(default, deserialize_with = "entries")]
    pub entries: Vec<ReadState>,
}

#[derive(Clone)]
pub struct Ready {
    pub user: User,
    pub users: Vec<User>,
    pub guilds: Vec<Guild>,
    pub guild_order: Vec<Option<u64>>,
    pub private_channels: Vec<Channel>,
    pub read_state: ReadStates,
    pub session_id: String,
    pub resume_gateway_url: Option<String>,
    pub user_settings_proto: Option<String>,
}

impl<'de> Deserialize<'de> for Ready {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            user: User,
            #[serde(default, deserialize_with = "entries")]
            users: Vec<User>,
            #[serde(default)]
            guilds: Vec<Value>,
            #[serde(default)]
            merged_members: Vec<Value>,
            #[serde(default, deserialize_with = "entries")]
            private_channels: Vec<Channel>,
            #[serde(default)]
            read_state: ReadStates,
            session_id: String,
            #[serde(default)]
            resume_gateway_url: Option<String>,
            #[serde(default)]
            user_settings_proto: Option<String>,
        }
        let wire = Wire::deserialize(d)?;
        let guild_order = wire
            .guilds
            .iter()
            .map(|guild| guild.get("id").and_then(id_value))
            .collect();
        let guilds = wire
            .guilds
            .into_iter()
            .enumerate()
            .filter_map(
                |(index, value)| match serde_json::from_value::<Guild>(value) {
                    Ok(mut guild) => {
                        if let Some(members) = wire.merged_members.get(index) {
                            guild.members = Some(parse_entries(members.clone()));
                        }
                        Some(guild)
                    }
                    Err(_) => {
                        log::warn!("Skipping malformed guild entry");
                        None
                    }
                },
            )
            .collect();
        Ok(Self {
            user: wire.user,
            users: wire.users,
            guilds,
            guild_order,
            private_channels: wire.private_channels,
            read_state: wire.read_state,
            session_id: wire.session_id,
            resume_gateway_url: wire.resume_gateway_url,
            user_settings_proto: wire.user_settings_proto,
        })
    }
}

#[derive(Clone, Default, Deserialize)]
pub struct Embed {
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<EmbedMedia>,
    #[serde(default)]
    pub image: Option<EmbedMedia>,
    #[serde(default)]
    pub video: Option<EmbedMedia>,
    #[serde(default)]
    pub provider: Option<EmbedProvider>,
    #[serde(default)]
    pub color: Option<u32>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<EmbedAuthor>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "entries")]
    pub fields: Vec<EmbedField>,
    #[serde(default, deserialize_with = "entries")]
    pub components: Vec<Component>,
    #[serde(default)]
    pub footer: Option<EmbedFooter>,
}

#[derive(Clone, Deserialize)]
pub struct EmbedAuthor {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub proxy_icon_url: Option<String>,
}
#[derive(Clone, Deserialize)]
pub struct EmbedField {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub inline: bool,
}
#[derive(Clone, Deserialize)]
pub struct EmbedFooter {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub proxy_icon_url: Option<String>,
}

#[derive(Clone, Default, Deserialize)]
pub struct EmbedMedia {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub flags: u64,
}

#[derive(Clone, Deserialize)]
pub struct EmbedProvider {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Clone, Default, Deserialize)]
pub struct Emoji {
    #[serde(default, deserialize_with = "optional_id")]
    pub id: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub animated: bool,
}

#[derive(Clone, Deserialize)]
pub struct Reaction {
    #[serde(default)]
    pub emoji: Emoji,
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub me: bool,
    #[serde(default)]
    pub me_burst: bool,
    #[serde(default)]
    pub burst_me: bool,
}

fn component_type<'de, D: Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    let kind = u8::deserialize(d)?;
    if matches!(kind, 1..=3 | 5..=14 | 17) {
        Ok(kind)
    } else {
        Err(serde::de::Error::custom("unknown component type"))
    }
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Deserialize)]
pub struct Component {
    #[serde(rename = "type", deserialize_with = "component_type")]
    pub kind: u8,
    #[serde(default, deserialize_with = "entries")]
    pub components: Vec<Component>,
    #[serde(default, deserialize_with = "default_value")]
    pub accessory: Option<Box<Component>>,
    #[serde(default)]
    pub style: u8,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub emoji: Option<Emoji>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub media: Option<EmbedMedia>,
    #[serde(default)]
    pub file: Option<EmbedMedia>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub spoiler: bool,
    #[serde(default, deserialize_with = "entries")]
    pub items: Vec<GalleryItem>,
    #[serde(default = "default_true")]
    pub divider: bool,
    #[serde(default)]
    pub spacing: u8,
    #[serde(default)]
    pub accent_color: Option<u32>,
}

#[derive(Clone, Deserialize)]
pub struct GalleryItem {
    #[serde(default)]
    pub media: EmbedMedia,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub spoiler: bool,
}

#[derive(Deserialize)]
pub struct ReactionEvent {
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    #[serde(deserialize_with = "snowflake")]
    pub message_id: u64,
    #[serde(default, deserialize_with = "optional_id")]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub emoji: Emoji,
}

#[derive(Deserialize)]
pub struct ReactionManyEvent {
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    #[serde(deserialize_with = "snowflake")]
    pub message_id: u64,
    #[serde(default, deserialize_with = "entries")]
    pub reactions: Vec<DebouncedReaction>,
}

#[derive(Deserialize)]
pub struct DebouncedReaction {
    #[serde(default, deserialize_with = "ids")]
    pub users: Vec<u64>,
    #[serde(default)]
    pub emoji: Emoji,
}

#[derive(Clone, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub flags: u64,
    #[serde(default, deserialize_with = "entries")]
    pub components: Vec<Component>,
    #[serde(default, deserialize_with = "entries")]
    pub reactions: Vec<Reaction>,
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    #[serde(default, deserialize_with = "optional_nonce")]
    pub nonce: Option<u64>,
    pub author: User,
    #[serde(deserialize_with = "timestamp")]
    pub timestamp: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, deserialize_with = "entries")]
    pub embeds: Vec<Embed>,
    #[serde(default, deserialize_with = "entries")]
    pub attachments: Vec<Attachment>,
    #[serde(default, deserialize_with = "entries")]
    pub sticker_items: Vec<Sticker>,
    #[serde(default, rename = "type")]
    pub kind: u8,
    #[serde(default)]
    pub edited_timestamp: Option<String>,
    #[serde(default, deserialize_with = "entries")]
    pub mentions: Vec<User>,
    #[serde(default, deserialize_with = "ids")]
    pub mention_roles: Vec<u64>,
    #[serde(default)]
    pub mention_everyone: bool,
    #[serde(default)]
    pub member: Option<PartialMember>,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default)]
    pub message_reference: Option<MessageReference>,
    #[serde(default, deserialize_with = "entries")]
    pub message_snapshots: Vec<MessageSnapshot>,
    #[serde(default, deserialize_with = "nullable")]
    pub referenced_message: Option<Option<Box<ReferencedMessage>>>,
}

#[derive(Clone, Deserialize)]
pub struct MessageUpdate {
    #[serde(default)]
    pub flags: Option<u64>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub components: Option<Vec<Component>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub reactions: Option<Vec<Reaction>>,
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    #[serde(default)]
    pub author: Option<User>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub embeds: Option<Vec<Embed>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub attachments: Option<Vec<Attachment>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub sticker_items: Option<Vec<Sticker>>,
    #[serde(default, rename = "type")]
    pub kind: Option<u8>,
    #[serde(default, deserialize_with = "nullable")]
    pub edited_timestamp: Option<Option<String>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub mentions: Option<Vec<User>>,
    #[serde(default, deserialize_with = "optional_ids")]
    pub mention_roles: Option<Vec<u64>>,
    #[serde(default)]
    pub mention_everyone: Option<bool>,
    #[serde(default)]
    pub member: Option<PartialMember>,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default, deserialize_with = "nullable")]
    pub message_reference: Option<Option<MessageReference>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub message_snapshots: Option<Vec<MessageSnapshot>>,
    #[serde(default, deserialize_with = "nullable")]
    pub referenced_message: Option<Option<Box<ReferencedMessage>>>,
}

#[derive(Clone, Default, Deserialize)]
pub struct PartialMember {
    #[serde(default, deserialize_with = "optional_id")]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default, deserialize_with = "nullable")]
    pub nick: Option<Option<String>>,
    #[serde(default, deserialize_with = "optional_ids")]
    pub roles: Option<Vec<u64>>,
    #[serde(default, deserialize_with = "nullable")]
    pub avatar: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub avatar_decoration_data: Option<Option<AvatarDecoration>>,
    #[serde(default, deserialize_with = "nullable")]
    pub display_name_styles: Option<Option<DisplayNameStyles>>,
    #[serde(default, deserialize_with = "nullable")]
    pub collectibles: Option<Option<Collectibles>>,
    #[serde(default)]
    pub presence: Option<Presence>,
}

impl PartialMember {
    pub fn id(&self) -> Option<u64> {
        self.user_id.or_else(|| self.user.as_ref().map(|u| u.id))
    }
}

impl From<&Member> for PartialMember {
    fn from(member: &Member) -> Self {
        Self {
            user_id: member.id(),
            user: member.user.clone(),
            nick: Some(member.nick.clone()),
            roles: Some(member.roles.clone()),
            avatar: Some(member.avatar.clone()),
            avatar_decoration_data: member.avatar_decoration_data.clone(),
            display_name_styles: member.display_name_styles.clone(),
            collectibles: member.collectibles.clone(),
            presence: None,
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
pub struct MessageReference {
    #[serde(default, deserialize_with = "optional_id")]
    pub message_id: Option<u64>,
    #[serde(default, deserialize_with = "optional_id")]
    pub channel_id: Option<u64>,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default, rename = "type")]
    pub kind: u8,
}

#[derive(Clone, Deserialize)]
pub struct MessageSnapshot {
    pub message: SnapshotMessage,
}

#[derive(Clone, Default, Deserialize)]
pub struct SnapshotMessage {
    #[serde(default)]
    pub content: String,
    #[serde(default, deserialize_with = "entries")]
    pub embeds: Vec<Embed>,
    #[serde(default, deserialize_with = "entries")]
    pub attachments: Vec<Attachment>,
    #[serde(default, deserialize_with = "entries")]
    pub sticker_items: Vec<Sticker>,
    #[serde(default, deserialize_with = "entries")]
    pub components: Vec<Component>,
    #[serde(default)]
    pub flags: u64,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub timestamp: Option<String>,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub edited_timestamp: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct TypingStart {
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(deserialize_with = "snowflake")]
    pub user_id: u64,
    #[serde(default)]
    pub member: Option<PartialMember>,
}

#[derive(Clone, Deserialize)]
pub struct ReferencedMessage {
    #[serde(default, deserialize_with = "entries")]
    pub components: Vec<Component>,
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    pub author: User,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub flags: u64,
    #[serde(default)]
    pub content: String,
    #[serde(default, deserialize_with = "entries")]
    pub attachments: Vec<Attachment>,
    #[serde(default, deserialize_with = "entries")]
    pub sticker_items: Vec<Sticker>,
    #[serde(default, deserialize_with = "entries")]
    pub embeds: Vec<Embed>,
    #[serde(default, deserialize_with = "entries")]
    pub message_snapshots: Vec<MessageSnapshot>,
}

#[derive(Clone, Default, Deserialize)]
pub struct Presence {
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default, deserialize_with = "optional_id")]
    pub user_id: Option<u64>,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub activities: Option<Vec<Activity>>,
}

#[derive(Clone, Default, Deserialize)]
pub struct Activity {
    #[serde(default, rename = "type")]
    pub kind: u8,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub details: Option<String>,
    #[serde(default)]
    pub emoji: Option<ActivityEmoji>,
}

#[derive(Clone, Default, Deserialize)]
pub struct ActivityEmoji {
    #[serde(default, deserialize_with = "optional_id")]
    pub id: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
}

fn entry_groups<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    d: D,
) -> Result<Vec<Vec<T>>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .cloned()
        .map(parse_entries)
        .collect())
}

#[derive(Default, Deserialize)]
pub struct MergedPresences {
    #[serde(default, deserialize_with = "entry_groups")]
    pub guilds: Vec<Vec<Presence>>,
    #[serde(default, deserialize_with = "entries")]
    pub friends: Vec<Presence>,
}

#[derive(Default, Deserialize)]
pub struct ReadySupplemental {
    #[serde(default)]
    pub guilds: Vec<Value>,
    #[serde(default, deserialize_with = "entry_groups")]
    pub merged_members: Vec<Vec<PartialMember>>,
    #[serde(default, deserialize_with = "default_value")]
    pub merged_presences: MergedPresences,
}

#[derive(Deserialize)]
pub struct MembersChunk {
    #[serde(deserialize_with = "snowflake")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "entries")]
    pub members: Vec<PartialMember>,
    #[serde(default, deserialize_with = "ids")]
    pub not_found: Vec<u64>,
    #[serde(default, deserialize_with = "entries")]
    pub presences: Vec<Presence>,
}

#[derive(Deserialize)]
pub struct MemberUpdate {
    #[serde(deserialize_with = "snowflake")]
    pub guild_id: u64,
    #[serde(flatten)]
    pub member: PartialMember,
}

#[derive(Deserialize)]
pub struct RoleUpdate {
    #[serde(deserialize_with = "snowflake")]
    pub guild_id: u64,
    pub role: Role,
}

#[derive(Default, Deserialize)]
pub struct MemberListUpdate {
    #[serde(default, deserialize_with = "snowflake")]
    pub guild_id: u64,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub member_count: u32,
    #[serde(default)]
    pub online_count: u32,
    #[serde(default, deserialize_with = "entries")]
    pub groups: Vec<MemberGroup>,
    #[serde(default, deserialize_with = "entries")]
    pub ops: Vec<MemberListOp>,
}

#[derive(Default, Deserialize)]
pub struct MemberGroup {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub count: u32,
}

#[derive(Default, Deserialize)]
pub struct MemberListOp {
    #[serde(default)]
    pub op: String,
    #[serde(default)]
    pub range: Option<[u32; 2]>,
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(default, deserialize_with = "entries")]
    pub items: Vec<MemberItem>,
    #[serde(default)]
    pub item: Option<MemberItem>,
}

#[derive(Default, Deserialize)]
pub struct MemberItem {
    #[serde(default)]
    pub group: Option<MemberGroup>,
    #[serde(default)]
    pub member: Option<PartialMember>,
}

#[derive(Clone, Default, Deserialize)]
pub struct Sticker {
    #[serde(default, deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub format_type: u8,
}

#[derive(Clone, Deserialize)]
pub struct Attachment {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub proxy_url: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub flags: u64,
}

#[derive(Deserialize)]
pub struct SettingsUpdate {
    pub settings: SettingsProto,
    #[serde(default)]
    pub partial: bool,
}

#[derive(Deserialize)]
pub struct SettingsProto {
    #[serde(rename = "type")]
    pub kind: u8,
    pub proto: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROFILE: &str = include_str!("fixtures/profile_modal.json");
    const PRODUCTS: &str = include_str!("fixtures/collectibles_products.json");

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
        let base = json!({"id":"1","channel_id":"2","author":{"id":"3"},"timestamp":"2026-10-22T00:00:00Z"});
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
        let update: MessageUpdate =
            serde_json::from_value(json!({"id":"1","channel_id":"2"})).unwrap();
        assert!(
            update.kind.is_none()
                && update.mentions.is_none()
                && update.referenced_message.is_none()
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
        let base = json!({"id":"1","channel_id":"2","author":{"id":"3"},"timestamp":"2026-10-22T00:00:00Z"});
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
        let base = json!({"id":"1","channel_id":"2","author":{"id":"3"},"timestamp":"2026-10-22T00:00:00Z"});
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
}
