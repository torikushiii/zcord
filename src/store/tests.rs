use super::*;
use crate::discord::settings_fixtures::{field, uint};
use crate::discord::settings_proto::FavoriteGif;
use crate::discord::{Event, settings_fixtures};
use crate::media::images::ImageSpec;
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{Datelike, Timelike};

use serde_json::json;

mod account;
mod activity_feed;
mod calls;
mod channels;
mod embeds;
mod expressions;
mod invites;
mod members;
mod messages;
mod notifications;
mod performance;
mod pins_threads;
mod reactions;
mod read_state;
mod settings;
mod voice;
mod webhooks;

pub(super) fn ready() -> model::Ready {
    serde_json::from_str(include_str!("../discord/fixtures/ready.json")).unwrap()
}

pub(super) fn messages() -> Vec<model::Message> {
    model::parse_entries(
        serde_json::from_str(include_str!("../discord/fixtures/messages.json")).unwrap(),
    )
}

pub(super) fn forum_thread(
    id: u64,
    forum_id: u64,
    name: &str,
    last_message: Option<u64>,
    created: &str,
    flags: u64,
    message_count: u32,
) -> serde_json::Value {
    json!({
        "id": id.to_string(),
        "type": 11,
        "guild_id": "200",
        "parent_id": forum_id.to_string(),
        "name": name,
        "owner_id": "1",
        "last_message_id": last_message.map(|id| id.to_string()),
        "message_count": message_count,
        "thread_metadata": {
            "archived": false,
            "locked": false,
            "create_timestamp": created
        },
        "flags": flags
    })
}

pub(super) fn forum_ready(
    threads: Vec<serde_json::Value>,
    read_last: Option<u64>,
    sort: Option<u8>,
) -> model::Ready {
    let mut payload: serde_json::Value =
        serde_json::from_str(include_str!("../discord/fixtures/ready.json")).unwrap();
    let guild = &mut payload["guilds"][0];
    guild["roles"][1]["permissions"] = json!("274877910016");
    let channels = guild["channels"].as_array_mut().unwrap();
    let forum = channels
        .iter_mut()
        .find(|channel| channel["id"] == "2007")
        .unwrap();
    forum["available_tags"] = json!([
        {"id":"4001","name":"Help","moderated":false,"emoji":{"name":"\u{1F9E0}"}},
        {"id":"4002","name":"Showcase","moderated":true,"emoji":{"id":"555","name":"party","animated":false}}
    ]);
    forum["default_sort_order"] = sort.map_or(serde_json::Value::Null, |sort| json!(sort));
    channels.push(json!({
        "id":"2009","type":16,"name":"media","position":3,"parent_id":"210"
    }));
    guild["threads"] = json!(threads);
    if let Some(read_last) = read_last {
        payload["read_state"]["entries"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"2007","last_message_id":read_last.to_string(),"mention_count":0}));
    }
    serde_json::from_value(payload).unwrap()
}

pub(super) fn social_fixture(key: &str) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(include_str!("../discord/fixtures/social.json"))
        .unwrap()[key]
        .clone()
}

pub(super) fn member_event(store: &mut Store, value: serde_json::Value) -> Change {
    store.apply(Event::MemberUpsert(Box::new(
        serde_json::from_value(value).unwrap(),
    )))
}

pub(super) fn role_event(store: &mut Store, value: serde_json::Value) -> Change {
    store.apply(Event::RoleUpsert(Box::new(
        serde_json::from_value(value).unwrap(),
    )))
}

pub(super) fn custom_emoji(id: u64, guild_id: u64, name: &str) -> CustomEmoji {
    CustomEmoji {
        id,
        name: name.to_owned(),
        animated: false,
        guild_id,
        available: true,
        roles: Vec::new(),
    }
}

pub(super) fn guild_emoji(id: u64, name: &str) -> model::GuildEmoji {
    model::GuildEmoji {
        id,
        name: name.to_owned(),
        animated: false,
        available: true,
        roles: Vec::new(),
        require_colons: true,
    }
}

pub(super) fn server_message(
    id: u64,
    channel_id: u64,
    author_id: u64,
    nonce: Option<u64>,
) -> model::Message {
    let mut value = json!({
        "id": id.to_string(), "channel_id": channel_id.to_string(),
        "author": {"id": author_id.to_string(), "username": "me"},
        "timestamp": "2026-10-22T12:00:00Z", "content": "hi"
    });
    if let Some(nonce) = nonce {
        value["nonce"] = json!(nonce.to_string());
    }
    serde_json::from_value(value).unwrap()
}

pub(super) fn favorite_gif(url: &str, order: u32) -> FavoriteGif {
    FavoriteGif {
        url: url.into(),
        src: format!("https://static.klipy.com/{order}.webp"),
        format: 1,
        width: 200,
        height: 150,
        order,
    }
}

pub(super) fn favorite_entry(gif: &FavoriteGif) -> Vec<u8> {
    let mut value = uint(1, u64::from(gif.format));
    value.extend(field(2, gif.src.as_bytes()));
    value.extend(uint(3, u64::from(gif.width)));
    value.extend(uint(4, u64::from(gif.height)));
    value.extend(uint(5, u64::from(gif.order)));
    let mut entry = field(1, gif.url.as_bytes());
    entry.extend(field(2, &value));
    entry
}

pub(super) fn frecency_proto(gifs: &[FavoriteGif], hide_tooltip: bool) -> Vec<u8> {
    let mut favorites = if hide_tooltip { uint(2, 1) } else { Vec::new() };
    for gif in gifs {
        favorites.extend(field(1, &favorite_entry(gif)));
    }
    let mut bytes = field(1, &uint(1, 3));
    bytes.extend(field(2, &favorites));
    bytes.extend(field(5, &field(1, &[])));
    bytes
}

pub(super) fn settings_update(kind: u8, proto: &[u8], partial: bool) -> Event {
    Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto {
            kind,
            proto: STANDARD.encode(proto),
        },
        partial,
    }))
}

#[test]
fn upload_limits_follow_nitro_and_boost_tiers() {
    const MIB: u64 = 1024 * 1024;
    let mut store = Store::from_ready(ready());
    for (premium_type, tier, expected) in [
        (None, 0, 20 * MIB),
        (Some(0), 0, 20 * MIB),
        (Some(1), 0, 50 * MIB),
        (Some(3), 0, 50 * MIB),
        (Some(2), 0, 500 * MIB),
        (Some(0), 1, 20 * MIB),
        (Some(0), 2, 50 * MIB),
        (Some(0), 3, 100 * MIB),
        (Some(1), 3, 100 * MIB),
        (Some(2), 3, 500 * MIB),
    ] {
        store.premium_type = premium_type;
        store
            .guilds
            .iter_mut()
            .find(|guild| guild.id == 200)
            .unwrap()
            .premium_tier = tier;
        assert_eq!(
            store.upload_limit(2002),
            expected,
            "{premium_type:?} with tier {tier}"
        );
    }
    store.premium_type = Some(2);
    assert_eq!(store.upload_limit(999_999), 500 * MIB);
}
