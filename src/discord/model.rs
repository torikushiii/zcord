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
    Option::<Value>::deserialize(d)?
        .map(|v| id_value(&v).ok_or_else(|| serde::de::Error::custom("invalid snowflake")))
        .transpose()
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
    let values = Vec::<Value>::deserialize(d)?;
    values
        .iter()
        .map(|v| id_value(v).ok_or_else(|| serde::de::Error::custom("invalid role id")))
        .collect()
}

fn timestamp<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    chrono::DateTime::parse_from_rfc3339(&value)
        .map_err(|_| serde::de::Error::custom("invalid timestamp"))?;
    Ok(value)
}

#[derive(Clone, Deserialize)]
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
    pub bot: bool,
}

#[derive(Clone, Deserialize)]
pub struct Member {
    #[serde(default, deserialize_with = "optional_id")]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default, deserialize_with = "ids")]
    pub roles: Vec<u64>,
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
    #[serde(deserialize_with = "snowflake")]
    pub permissions: u64,
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
    pub roles: Option<Vec<Role>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub members: Option<Vec<Member>>,
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
        if update.channels.is_some() {
            self.channels.clone_from(&update.channels);
        }
        if update.roles.is_some() {
            self.roles.clone_from(&update.roles);
        }
        if update.members.is_some() {
            self.members.clone_from(&update.members);
        }
        if update.unavailable.is_some() {
            self.unavailable = update.unavailable;
        }
    }
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
    pub private_channels: Vec<Channel>,
    pub read_state: ReadStates,
    pub session_id: String,
    pub resume_gateway_url: Option<String>,
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
        }
        let wire = Wire::deserialize(d)?;
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
            private_channels: wire.private_channels,
            read_state: wire.read_state,
            session_id: wire.session_id,
            resume_gateway_url: wire.resume_gateway_url,
        })
    }
}

#[derive(Clone, Default, Deserialize)]
pub struct Embed {
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
    #[serde(default)]
    pub footer: Option<EmbedFooter>,
}

#[derive(Clone, Deserialize)]
pub struct EmbedAuthor {
    #[serde(default)]
    pub name: String,
}
#[derive(Clone, Deserialize)]
pub struct EmbedField {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: String,
}
#[derive(Clone, Deserialize)]
pub struct EmbedFooter {
    #[serde(default)]
    pub text: String,
}

#[derive(Clone, Deserialize)]
pub struct Message {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    pub author: User,
    #[serde(deserialize_with = "timestamp")]
    pub timestamp: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, deserialize_with = "entries")]
    pub embeds: Vec<Embed>,
}

#[derive(Clone, Deserialize)]
pub struct MessageUpdate {
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
}
