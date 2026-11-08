use super::guilds::Channel;
use super::messages::Timestamp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationshipKind {
    Friend,
    Blocked,
    Incoming,
    Outgoing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relationship {
    pub user_id: u64,
    pub kind: RelationshipKind,
    pub nickname: Option<String>,
    pub since: Option<Timestamp>,
    pub spam: bool,
    pub ignored: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FriendSuggestion {
    pub user_id: u64,
    pub reason: Option<String>,
}

pub struct PrivateChannel {
    pub channel: Channel,
    pub group: bool,
    pub recipient_ids: Vec<u64>,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub owner_id: Option<u64>,
    pub message_request: bool,
    pub spam: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestPreview {
    pub author_id: u64,
    pub content: String,
    pub timestamp: Timestamp,
}
