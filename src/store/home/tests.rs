use crate::discord::Event;
use crate::discord::model;
use crate::store::{Change, PrivateChannel, RelationshipKind, Status, Store};
use serde_json::json;

fn ready() -> model::Ready {
    serde_json::from_str(include_str!(
        "../../../crates/discord/src/fixtures/home.json"
    ))
    .unwrap()
}

fn store() -> Store {
    Store::from_ready(ready())
}

fn message(id: u64, channel_id: u64, author: u64) -> model::Message {
    serde_json::from_value(json!({
        "id": id.to_string(),
        "channel_id": channel_id.to_string(),
        "author": {"id": author.to_string(), "username": "someone"},
        "timestamp": "2026-10-22T12:00:00Z",
        "content": "hello"
    }))
    .unwrap()
}

fn dm_ids(list: &[&PrivateChannel]) -> Vec<u64> {
    list.iter().map(|private| private.channel.id).collect()
}

#[test]
fn ready_parses_home_state() {
    let ready = ready();
    assert_eq!(ready.relationships.len(), 8);
    assert_eq!(ready.friend_suggestion_count, 2);
    let request = ready
        .private_channels
        .iter()
        .find(|channel| channel.id == 904)
        .unwrap();
    assert_eq!(request.is_message_request, Some(true));
    let group = ready
        .private_channels
        .iter()
        .find(|channel| channel.id == 902)
        .unwrap();
    assert_eq!(group.icon, Some(Some("group-icon".to_owned())));
}

#[test]
fn dm_list_orders_by_last_activity_and_hides_requests() {
    let store = store();
    assert_eq!(dm_ids(&store.dm_list()), [906, 901, 902, 900, 903]);
    let ids = dm_ids(&store.dm_list());
    assert!(!ids.contains(&904));
    assert!(!ids.contains(&905));
}

#[test]
fn message_requests_are_listed_separately() {
    let store = store();
    assert_eq!(dm_ids(&store.message_requests()), [904]);
    assert_eq!(store.message_request_count(), 1);
}

#[test]
fn spam_requests_are_listed_and_counted_separately() {
    let store = store();
    assert_eq!(dm_ids(&store.spam_requests()), [905]);
    assert_eq!(store.spam_request_count(), 1);
    assert_eq!(store.message_request_count(), 1);
}

#[test]
fn request_previews_register_authors_and_follow_new_messages() {
    let mut store = store();
    let previews: Vec<model::MessageRequestPreview> = serde_json::from_value(json!([{
        "channel_id": "904",
        "message_preview": {
            "id": "650",
            "channel_id": "904",
            "author": {"id": "14", "username": "dave", "global_name": "Dave"},
            "timestamp": "2025-02-03T04:05:06Z",
            "content": "wanna play?"
        }
    }]))
    .unwrap();
    store.set_request_previews(previews);
    assert_eq!(store.user(14).unwrap().name, "Dave");
    let preview = store.request_preview(904).unwrap();
    assert_eq!(preview.author_id, 14);
    assert_eq!(preview.content, "wanna play?");

    store.apply(Event::MessageCreate(Box::new(message(651, 904, 17))));
    let preview = store.request_preview(904).unwrap();
    assert_eq!(preview.author_id, 17);
    assert_eq!(preview.content, "hello");
}

#[test]
fn accepting_and_removing_requests_updates_the_lists() {
    let mut store = store();
    assert!(matches!(
        store.accept_request_locally(904),
        Change::Structure
    ));
    assert!(dm_ids(&store.dm_list()).contains(&904));
    assert_eq!(store.message_request_count(), 0);
    assert!(!store.private_channel(904).unwrap().message_request);

    assert!(matches!(
        store.remove_private_locally(905),
        Change::Structure
    ));
    assert!(store.private_channel(905).is_none());
    assert_eq!(store.spam_request_count(), 0);
    assert!(store.spam_requests().is_empty());
}

#[test]
fn relationships_split_friends_and_pending() {
    let store = store();
    assert_eq!(store.pending_count(), 1);
    assert_eq!(store.pending_requests(), [14, 17]);
    assert_eq!(store.friends(), [10, 11, 12]);
    assert_eq!(
        store.relationship(10).unwrap().kind,
        RelationshipKind::Friend
    );
    assert!(!store.friends().contains(&13));
}

#[test]
fn home_badge_sums_dm_mentions_and_requests() {
    let store = store();
    assert_eq!(store.dm_mentions, 2);
    assert_eq!(store.pending_count(), 1);
    assert_eq!(store.message_request_count(), 1);
    assert_eq!(store.home_badge(), 4);
}

#[test]
fn friend_suggestions_follow_fetches_events_and_relationships() {
    let mut store = store();
    assert!(!store.friend_suggestions_loaded());
    assert_eq!(store.friend_suggestion_count(), 2);

    let suggestion = |id: &str, reason: &str| {
        json!({
            "suggested_user": {"id": id, "username": format!("user{id}")},
            "reasons": [{"type": 1, "platform_type": "steam", "name": reason}]
        })
    };
    let fetched: Vec<model::FriendSuggestion> =
        serde_json::from_value(json!([suggestion("30", "steamy"), suggestion("31", "")])).unwrap();
    store.set_friend_suggestions(fetched);
    assert!(store.user(30).is_some());
    let list = store.friend_suggestions();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].reason.as_deref(), Some("steamy"));
    assert_eq!(list[1].reason, None);

    let created: model::FriendSuggestion = serde_json::from_value(suggestion("32", "new")).unwrap();
    store.apply(Event::FriendSuggestionCreate(Box::new(created)));
    assert_eq!(store.friend_suggestion_count(), 3);

    store.apply(Event::FriendSuggestionDelete { user_id: 31 });
    assert_eq!(store.friend_suggestion_count(), 2);

    let outgoing: model::Relationship =
        serde_json::from_value(json!({"id": "30", "type": 4})).unwrap();
    store.apply(Event::RelationshipAdd(Box::new(outgoing)));
    let ids: Vec<u64> = store
        .friend_suggestions()
        .iter()
        .map(|suggestion| suggestion.user_id)
        .collect();
    assert_eq!(ids, [32]);
}

#[test]
fn online_friends_follow_presence() {
    let mut store = store();
    let supplemental: model::ReadySupplemental = serde_json::from_value(json!({
        "merged_presences": {
            "friends": [
                {"user_id": "10", "status": "online"},
                {"user_id": "12", "status": "idle"}
            ]
        }
    }))
    .unwrap();
    store.apply(Event::ReadySupplemental(Box::new(supplemental)));
    assert_eq!(store.online_friends(), [10, 12]);
    assert!(store.user(12).unwrap().status == Status::Idle);
}

#[test]
fn active_now_includes_friends_and_close_contacts() {
    let mut store = store();
    let supplemental: model::ReadySupplemental = serde_json::from_value(json!({
        "merged_presences": {
            "friends": [
                {"user_id": "10", "status": "online", "activities": [{"type": 0, "name": "Example Game"}]},
                {"user_id": "11", "status": "online", "activities": []}
            ],
            "guilds": [[
                {"user_id": "40", "status": "online", "activities": [{"type": 0, "name": "Example Action Game"}]},
                {"user_id": "41", "status": "dnd", "activities": [{"type": 0, "name": "Example Multiplayer Game"}]},
                {"user_id": "42", "status": "idle", "activities": [{"type": 4, "name": "Custom Status", "state": "hi"}]},
                {"user_id": "43", "status": "dnd", "activities": [{"type": 2, "name": "Spotify", "details": "Song", "state": "Artist"}]},
                {"user_id": "44", "status": "offline", "activities": [{"type": 0, "name": "Hidden"}]}
            ]]
        }
    }))
    .unwrap();
    store.apply(Event::ReadySupplemental(Box::new(supplemental)));
    assert_eq!(store.active_now(), [10]);

    let affinities: Vec<model::UserAffinity> = serde_json::from_value(json!([
        {"other_user_id": "40", "communication_rank": 15},
        {"other_user_id": "41", "communication_rank": 16},
        {"other_user_id": "43"}
    ]))
    .unwrap();
    store.set_user_affinities(&affinities);
    let mut active = store.active_now();
    active.sort_unstable();
    assert_eq!(active, [10, 40, 43]);
}

#[test]
fn private_channel_names_use_group_name_or_recipients() {
    let store = store();
    assert_eq!(store.private_channel_name(900), "Alice");
    assert_eq!(store.private_channel_name(902), "Raid night");
    assert_eq!(store.private_channel_name(903), "Alice, bob, Carol");
    assert_eq!(store.private_channel_name(906), "Frank");
}

#[test]
fn dm_channel_lookup_uses_single_recipient() {
    let store = store();
    assert_eq!(store.dm_channel_with(10), Some(900));
    assert_eq!(store.dm_channel_with(13), None);
}

#[test]
fn dm_messages_use_the_network_channel_path_and_badges() {
    let mut store = store();
    assert!(store.private_channel(900).unwrap().channel.unread);
    assert_eq!(store.private_channel(900).unwrap().channel.mentions, 2);
    assert_eq!(store.dm_mentions, 2);

    store.set_messages(900, vec![message(490, 900, 10), message(500, 900, 10)]);
    assert_eq!(store.network_channel(900).unwrap().messages.len(), 2);
    assert!(store.at_present(900));

    assert!(matches!(
        store.apply(Event::MessageCreate(Box::new(message(501, 900, 10)))),
        Change::MessageInserted {
            channel_id: 900,
            index: 2
        }
    ));
    assert_eq!(store.network_channel(900).unwrap().messages.len(), 3);

    store.apply(Event::MessageCreate(Box::new(message(701, 901, 11))));
    let other = &store.private_channel(901).unwrap().channel;
    assert!(other.unread);
    assert_eq!(other.mentions, 1);

    store.mark_channel_read(0, 900);
    let channel = &store.private_channel(900).unwrap().channel;
    assert!(!channel.unread);
    assert_eq!(channel.mentions, 0);
    assert_eq!(store.dm_mentions, 1);

    store.apply(Event::MessageCreate(Box::new(message(10_000, 903, 10))));
    assert_eq!(store.dm_list().first().map(|dm| dm.channel.id), Some(903));
}

#[test]
fn relationship_events_update_friends_and_pending() {
    let mut store = store();
    let added: model::Relationship = serde_json::from_value(json!({
        "id": "20",
        "type": 3,
        "user": {"id": "20", "username": "newbie", "global_name": "Newbie"}
    }))
    .unwrap();
    assert!(matches!(
        store.apply(Event::RelationshipAdd(Box::new(added))),
        Change::Structure
    ));
    assert!(store.user(20).is_some());
    assert_eq!(store.pending_count(), 2);

    let update: model::RelationshipUpdate =
        serde_json::from_value(json!({"id": "20", "type": 1})).unwrap();
    store.apply(Event::RelationshipUpdate(Box::new(update)));
    assert!(store.friends().contains(&20));

    let update: model::RelationshipUpdate =
        serde_json::from_value(json!({"id": "11", "nickname": null})).unwrap();
    store.apply(Event::RelationshipUpdate(Box::new(update)));
    assert!(store.relationship(11).unwrap().nickname.is_none());
    assert_eq!(store.friend_name(11), "bob");

    store.apply(Event::RelationshipRemove { id: 10 });
    assert!(!store.friends().contains(&10));
    assert!(store.relationship(10).is_none());
}

#[test]
fn channel_upserts_move_requests_and_register_recipients() {
    let mut store = store();
    let update: model::Channel = serde_json::from_value(json!({
        "id": "904",
        "type": 1,
        "is_message_request": false
    }))
    .unwrap();
    store.apply(Event::ChannelUpsert(Box::new(update)));
    assert!(dm_ids(&store.dm_list()).contains(&904));
    assert_eq!(store.message_request_count(), 0);

    let created: model::Channel = serde_json::from_value(json!({
        "id": "907",
        "type": 1,
        "recipients": [{"id": "30", "username": "zed"}]
    }))
    .unwrap();
    store.apply(Event::ChannelUpsert(Box::new(created)));
    assert!(store.user(30).is_some());
    assert_eq!(store.dm_channel_with(30), Some(907));
}
