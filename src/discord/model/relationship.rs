use super::serde_helpers::*;
use super::*;
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct Relationship {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, rename = "type")]
    pub kind: u8,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub is_spam_request: bool,
    #[serde(default)]
    pub user_ignored: bool,
    #[serde(default)]
    pub user: Option<User>,
}

/// An entry of `GET /friend-suggestions`, also the `FRIEND_SUGGESTION_CREATE` payload.
#[derive(Clone, Deserialize)]
pub struct FriendSuggestion {
    pub suggested_user: User,
    #[serde(default, deserialize_with = "entries")]
    pub reasons: Vec<FriendSuggestionReason>,
}

/// Why Discord suggests someone; `name` is the matched name, such as their Steam name.
#[derive(Clone, Deserialize)]
pub struct FriendSuggestionReason {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct RelationshipUpdate {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, rename = "type")]
    pub kind: Option<u8>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub nickname: Option<Option<String>>,
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub is_spam_request: Option<bool>,
    #[serde(default)]
    pub user_ignored: Option<bool>,
}
