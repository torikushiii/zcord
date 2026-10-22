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
    pub discriminator: Option<String>,
    #[serde(default)]
    pub bot: bool,
    #[serde(default, deserialize_with = "nullable")]
    pub primary_guild: Option<Option<PrimaryGuild>>,
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
        if update.threads.is_some() {
            self.threads.clone_from(&update.threads);
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
    #[serde(default, deserialize_with = "entries")]
    pub attachments: Vec<Attachment>,
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
    #[serde(default, deserialize_with = "nullable")]
    pub referenced_message: Option<Option<Box<ReferencedMessage>>>,
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
    #[serde(default, deserialize_with = "optional_entries")]
    pub attachments: Option<Vec<Attachment>>,
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
pub struct ReferencedMessage {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    pub author: User,
    #[serde(default)]
    pub content: String,
    #[serde(default, deserialize_with = "entries")]
    pub attachments: Vec<Attachment>,
    #[serde(default, deserialize_with = "entries")]
    pub embeds: Vec<Embed>,
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
}
