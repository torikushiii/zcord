use super::*;

pub enum Event {
    ReadySupplemental(Box<model::ReadySupplemental>),
    PresenceUpdate(Box<model::Presence>),
    SessionsReplace(Vec<model::Session>),
    MembersChunk(Box<model::MembersChunk>),
    MemberListUpdate(Box<model::MemberListUpdate>),
    MemberUpsert(Box<model::MemberUpdate>),
    MemberRemove(Box<model::MemberUpdate>),
    RoleUpsert(Box<model::RoleUpdate>),
    RoleDelete { guild_id: u64, role_id: u64 },
    UserUpdate(Box<model::UserUpdate>),
    Ready(Box<model::Ready>),
    SettingsUpdate(Box<model::SettingsUpdate>),
    MessageCreate(Box<model::Message>),
    MessageUpdate(Box<model::MessageUpdate>),
    TypingStart(Box<model::TypingStart>),
    MessageReactionAdd(model::ReactionEvent),
    MessageReactionRemove(model::ReactionEvent),
    MessageReactionRemoveAll(model::ReactionEvent),
    MessageReactionRemoveEmoji(model::ReactionEvent),
    MessageReactionAddMany(model::ReactionManyEvent),
    MessageDelete { channel_id: u64, id: u64 },
    ChannelUpsert(Box<model::Channel>),
    ChannelDelete { id: u64 },
    RelationshipAdd(Box<model::Relationship>),
    RelationshipUpdate(Box<model::RelationshipUpdate>),
    RelationshipRemove { id: u64 },
    ThreadListSync(Box<model::ThreadListSync>),
    GuildUpsert(Box<model::Guild>),
    GuildEmojisUpdate(model::GuildEmojisUpdate),
    GuildDelete { id: u64 },
    MessageAck { channel_id: u64, message_id: u64 },
    VoiceStateUpdate(Box<model::VoiceState>),
    ChannelInfo(model::ChannelInfo),
    VoiceChannelStartTimeUpdate(model::VoiceChannelStartTimeUpdate),
    Voice(VoiceEvent),
    Reconnecting,
    Resumed,
    Fatal(FatalError),
}

#[derive(Clone, Debug, PartialEq)]
pub enum VoiceEvent {
    Connecting {
        guild_id: u64,
        channel_id: u64,
    },
    Connected {
        guild_id: u64,
        channel_id: u64,
    },
    Disconnected {
        reason: Option<String>,
    },
    Speaking {
        user_id: u64,
        speaking: bool,
    },
    PrivacyCode(Option<String>),
    /// Round trip of the voice gateway's heartbeat, in milliseconds.
    Ping(u32),
    /// Voice server host, without a scheme or port.
    Server(String),
    /// Outbound packet loss from an RTCP receiver report, in percent.
    PacketLoss(f32),
}

#[derive(Debug)]
pub enum FatalError {
    InvalidToken,
    Other(String),
}
