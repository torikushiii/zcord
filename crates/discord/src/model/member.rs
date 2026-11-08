use super::serde_helpers::*;
use super::*;
use serde::Deserialize;

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
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub unicode_emoji: Option<String>,
}

#[derive(Clone, Default, Deserialize)]
pub struct RoleColors {
    #[serde(default)]
    pub primary_color: u32,
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
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub join_timestamp: Option<String>,
    #[serde(default)]
    pub flags: Option<u32>,
    #[serde(default)]
    pub muted: Option<bool>,
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
            join_timestamp: None,
            flags: None,
            muted: None,
        }
    }
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
