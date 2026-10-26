use super::serde_helpers::*;
use super::*;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

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
    pub sessions: Vec<Session>,
    pub relationships: Vec<Relationship>,
    pub friend_suggestion_count: u32,
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
            #[serde(default, deserialize_with = "entries")]
            sessions: Vec<Session>,
            #[serde(default, deserialize_with = "entries")]
            relationships: Vec<Relationship>,
            #[serde(default)]
            friend_suggestion_count: u32,
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
                        guild.fill_voice_state_guilds();
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
            sessions: wire.sessions,
            relationships: wire.relationships,
            friend_suggestion_count: wire.friend_suggestion_count,
        })
    }
}
