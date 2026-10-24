use super::*;
use crate::discord::{Event, settings_fixtures};
use crate::images::ImageSpec;
use chrono::{Datelike, Timelike};

use serde_json::json;

fn ready() -> model::Ready {
    serde_json::from_str(include_str!("../discord/fixtures/ready.json")).unwrap()
}
fn messages() -> Vec<model::Message> {
    model::parse_entries(
        serde_json::from_str(include_str!("../discord/fixtures/messages.json")).unwrap(),
    )
}

fn forum_thread(
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

fn forum_ready(
    threads: Vec<serde_json::Value>,
    read_last: Option<u64>,
    sort: Option<u8>,
) -> model::Ready {
    let mut payload: serde_json::Value =
        serde_json::from_str(include_str!("../discord/fixtures/ready.json")).unwrap();
    let guild = &mut payload["guilds"][0];
    guild["roles"][1]["permissions"] = json!("3072");
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

#[test]
fn component_and_formatting_fixtures_map_to_view() {
    let source: model::Message =
        serde_json::from_str(include_str!("fixtures/components_v2.json")).unwrap();
    let message = map_message(&source).unwrap();
    assert!(message.embeds.is_empty());
    let [
        Component::Container {
            accent,
            spoiler,
            components,
        },
    ] = message.components.as_slice()
    else {
        panic!("container")
    };
    assert_eq!(*accent, None);
    assert!(!spoiler);
    let [
        Component::Section {
            components,
            accessory,
        },
        Component::Row(buttons),
    ] = components.as_slice()
    else {
        panic!("section and row")
    };
    assert!(
        matches!(&components[..], [Component::Text(text)] if text.starts_with("## SAMPLE NOTICE"))
    );
    let Component::Thumbnail {
        media,
        description,
        spoiler,
    } = accessory.as_deref().unwrap()
    else {
        panic!("thumbnail")
    };
    assert_eq!(
        media.url,
        source.components[0].components[0]
            .accessory
            .as_ref()
            .unwrap()
            .media
            .as_ref()
            .unwrap()
            .proxy_url
            .as_ref()
            .unwrap()
            .as_str()
    );
    assert!(
        media
            .url
            .starts_with("https://images-ext-1.discordapp.net/")
    );
    assert_eq!((media.width, media.height), (256, 256));
    assert!(description.is_none() && !spoiler);
    let [
        Component::Button {
            style,
            label,
            emoji,
            disabled,
            url,
        },
    ] = buttons.as_slice()
    else {
        panic!("button")
    };
    assert_eq!(*style, 2);
    assert_eq!(label.as_deref(), Some("Actions: 12"));
    assert_eq!(emoji.as_ref().unwrap().name, "\u{1f36f}");
    assert!(!disabled && url.is_none());
    assert_eq!(message.reactions.len(), 5);
    for (wire, view) in source.reactions.iter().zip(&message.reactions) {
        assert_eq!(wire.count, view.count);
        assert_eq!(wire.emoji.id, view.emoji.id);
        assert_eq!(wire.emoji.name.as_deref(), Some(view.emoji.name.as_str()));
    }
    let animated = &message.reactions[4];
    assert_eq!(animated.emoji.name, "sample_animated");
    assert_eq!(animated.emoji.id, Some(1191168972947456014));
    assert!(animated.emoji.animated);
    assert_eq!(animated.count, 4);
    let mut source = source;
    source.reactions[0].me_burst = true;
    source.reactions[1].burst_me = true;
    source.reactions[2].me = true;
    assert!(
        map_message(&source).unwrap().reactions[..3]
            .iter()
            .all(|r| r.me)
    );

    let source: model::Message =
        serde_json::from_str(include_str!("fixtures/formatting.json")).unwrap();
    let message = map_message(&source).unwrap();
    assert_eq!(message.content, source.content);
    assert!(message.embeds.is_empty() && message.components.is_empty());
    assert_eq!(message.reactions.len(), 3);
    assert_eq!(message.reactions[1].emoji.name, "sample_icon");
}

#[test]
fn malformed_rich_entries_never_discard_a_message() {
    let mut value = json!({
        "id":"1", "channel_id":"2", "author":{"id":"3"},
        "timestamp":"2026-04-09T10:24:21Z", "content":"intact",
        "embeds":[false, {"title":77}, {"title":"survives", "fields":[false, {"name":"n","value":"v"}]}],
        "components":[{"type":99}, null, {"type":17,"components":[{"type":99},{"type":9,"components":[false,{"type":10,"content":"intact"}],"accessory":{"type":99}}]},
            {"type":12,"items":[false,{"media":7},{"media":{"url":"https://cdn.discordapp.com/a"}}]}],
        "reactions":[false,{"count":"bad"},{"emoji":{"name":"ok"},"count":2}]
    });
    let source: model::Message = serde_json::from_value(value.clone()).unwrap();
    let mapped = map_message(&source).unwrap();
    assert_eq!(mapped.content, "intact");
    assert_eq!(mapped.embeds.len(), 1);
    assert_eq!(mapped.embeds[0].fields.len(), 1);
    assert_eq!(mapped.reactions.len(), 1);
    assert_eq!(mapped.components.len(), 2);
    let Component::Container { components, .. } = &mapped.components[0] else {
        panic!("container")
    };
    assert!(
        matches!(&components[..], [Component::Section { accessory: None, components }] if matches!(&components[..], [Component::Text(text)] if text == "intact"))
    );
    assert!(matches!(&mapped.components[1], Component::Gallery(items) if items.len() == 1));
    for field in ["embeds", "components", "reactions"] {
        value[field] = json!({"not":"an array"});
    }
    let source: model::Message = serde_json::from_value(value).unwrap();
    assert!(
        source.embeds.is_empty() && source.components.is_empty() && source.reactions.is_empty()
    );
}

#[test]
fn embeds_map_kinds_text_fields_timestamps_and_proxy_only_media() {
    let wire: model::Embed = serde_json::from_value(json!({
        "type":"rich", "url":"https://example.com/post", "title":"Title", "description":"Description", "color":123,
        "timestamp":"2026-04-09T10:24:21Z", "provider":{"name":"Provider","url":"https://example.com"},
        "author":{"name":"Author","url":"https://example.com/author","icon_url":"https://evil.test/icon", "proxy_icon_url":"https://images-ext-2.discordapp.net/icon"},
        "footer":{"text":"Footer","icon_url":"https://cdn.discordapp.com/footer"},
        "fields":[{"name":"n","value":"v","inline":true}],
        "thumbnail":{"url":"https://evil.test/thumb","proxy_url":"https://images-ext-1.discordapp.net/thumb","width":20,"height":30},
        "image":{"url":"https://evil.test/image"}, "video":{"url":"https://media.discordapp.net/video"}
    })).unwrap();
    let embed = mapping::map_embed(&wire);
    assert!(embed.kind == EmbedKind::Rich);
    assert_eq!(
        embed.title.as_ref().unwrap().url.as_deref(),
        Some("https://example.com/post")
    );
    assert_eq!(embed.description.as_deref(), Some("Description"));
    assert_eq!(embed.color, Some(123));
    assert_eq!(embed.provider.as_ref().unwrap().text, "Provider");
    assert_eq!(
        embed.author.as_ref().unwrap().icon.as_ref().unwrap().width,
        0
    );
    assert_eq!(
        embed.footer.as_ref().unwrap().icon.as_ref().unwrap().height,
        0
    );
    assert!(embed.fields[0].inline);
    assert_eq!(
        embed.thumbnail.as_ref().unwrap().url,
        "https://images-ext-1.discordapp.net/thumb"
    );
    assert!(embed.images.is_empty());
    assert_eq!(embed.video.as_ref().unwrap().width, 0);
    assert!(embed.timestamp == local_timestamp("2026-04-09T10:24:21Z"));
    for (kind, expected) in [
        (None, EmbedKind::Rich),
        (Some(""), EmbedKind::Rich),
        (Some("image"), EmbedKind::Image),
        (Some("gifv"), EmbedKind::Gifv),
        (Some("video"), EmbedKind::Video),
        (Some("components"), EmbedKind::Components),
        (Some("article"), EmbedKind::Link),
        (Some("link"), EmbedKind::Link),
        (Some("future"), EmbedKind::Link),
    ] {
        let mut wire = model::Embed {
            kind: kind.map(str::to_owned),
            ..Default::default()
        };
        assert!(mapping::map_embed(&wire).kind == expected);
        wire.color = Some(0);
        if matches!(kind, Some("article" | "link" | "future")) {
            assert!(mapping::map_embed(&wire).kind == EmbedKind::Rich);
            wire.color = None;
            wire.fields = vec![serde_json::from_value(json!({})).unwrap()];
            assert!(mapping::map_embed(&wire).kind == EmbedKind::Rich);
        }
    }
    let empty: model::Embed = serde_json::from_value(json!({"title":"","url":"","description":"","author":{"name":""},"provider":{"name":""},"footer":{"text":""},"timestamp":"bad"})).unwrap();
    let empty = mapping::map_embed(&empty);
    assert!(
        empty.title.is_none()
            && empty.url.is_none()
            && empty.description.is_none()
            && empty.author.is_none()
            && empty.provider.is_none()
            && empty.footer.is_none()
            && empty.timestamp.is_none()
    );
    for (url, proxy, allowed) in [
        ("https://evil.test/image", None, false),
        ("https://images-ext-1.discordapp.net/a", None, false),
        ("https://cdn.discordapp.com/a", None, true),
        (
            "https://media.discordapp.net/a",
            Some("https://evil.test/proxy"),
            false,
        ),
        (
            "https://evil.test/a",
            Some("https://images-ext-1.discordapp.net/a"),
            true,
        ),
        ("https://user@cdn.discordapp.com/a", None, false),
        ("http://cdn.discordapp.com/a", None, false),
    ] {
        let wire: model::Embed =
            serde_json::from_value(json!({"image":{"url":url,"proxy_url":proxy}})).unwrap();
        assert_eq!(!mapping::map_embed(&wire).images.is_empty(), allowed);
    }
}

#[test]
fn reddit_component_embeds_preserve_text_galleries_and_link_buttons() {
    let source: model::Message =
        serde_json::from_str(include_str!("fixtures/reddit_embeds.json")).unwrap();
    let message = map_message(&source).unwrap();
    assert!(message.components.is_empty());
    assert_eq!(message.content, source.content);
    assert_eq!(message.embeds.len(), 3);
    for embed in &message.embeds[..2] {
        assert!(embed.kind == EmbedKind::Components);
        assert!(embed.title.is_none());
        assert_eq!(embed.components.len(), 1);
    }
    let Component::Container {
        accent, components, ..
    } = &message.embeds[0].components[0]
    else {
        panic!("vxReddit container expected");
    };
    assert_eq!(*accent, Some(16729344));
    assert!(matches!(&components[0], Component::Text(text) if text == "-# vxReddit"));
    assert!(matches!(&components[1], Component::Text(text) if text.contains("Sample multi-image post")));
    let Component::Gallery(items) = &components[2] else {
        panic!("gallery expected");
    };
    assert_eq!(items.len(), 1);
    assert_eq!((items[0].media.width, items[0].media.height), (1080, 1350));
    assert!(
        items[0]
            .media
            .url
            .starts_with("https://images-ext-1.discordapp.net/")
    );
    assert_eq!(
        items[0].media.source,
        "https://i.redd.it/sample-image-one.jpeg"
    );
    assert!(matches!(&components[3], Component::Row(buttons)
        if matches!(&buttons[..], [Component::Button { style: 5, label: Some(label), url: Some(url), .. }]
            if label == "View post" && url == "https://www.reddit.com/comments/samplePostOne")));
    let Component::Container { components, .. } = &message.embeds[1].components[0] else {
        panic!("Reddit container expected");
    };
    assert!(
        matches!(&components[2], Component::Gallery(items) if items.len() == 1 && items[0].spoiler)
    );
    assert!(message.embeds[2].kind == EmbedKind::Rich);
    assert_eq!(
        message.embeds[2].title.as_ref().unwrap().text,
        "Existing standard embed"
    );
    assert_eq!(message.embeds[2].images.len(), 1);

    let snapshot: model::SnapshotMessage = serde_json::from_value(json!({
        "embeds": serde_json::from_str::<serde_json::Value>(include_str!("fixtures/reddit_embeds.json")).unwrap()["embeds"].clone()
    })).unwrap();
    let forward = mapping::map_snapshot(&snapshot, Some(2002), Some(200));
    assert_eq!(forward.embeds[0].components.len(), 1);
    assert!(mapping::map_embeds(&source.embeds, 4).is_empty());
}

#[test]
fn component_embed_updates_are_not_lost_or_merged_into_other_cards() {
    let mut store = Store::from_ready(ready());
    let source = serde_json::from_str(include_str!("fixtures/reddit_embeds.json")).unwrap();
    store.set_messages(2002, vec![source]);
    let component = json!({
        "type":"components", "url":"https://www.reddit.com/comments/samplePostTwo",
        "components":[false, {"type":17,"components":[{"type":99}, {"type":10,"content":"Updated Reddit preview"}]}]
    });
    let update = json!({
        "id":"500", "channel_id":"2002",
        "embeds":[{"type":"rich","url":"https://www.reddit.com/comments/samplePostTwo","title":"Legacy preview"}, component.clone(), component]
    });
    assert_eq!(
        store.apply(Event::MessageUpdate(Box::new(
            serde_json::from_value(update).unwrap()
        ))),
        Change::MessageChanged {
            channel_id: 2002,
            index: 0
        }
    );
    let message = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages[0];
    assert_eq!(message.embeds.len(), 3);
    for embed in &message.embeds[1..] {
        assert!(embed.kind == EmbedKind::Components);
        let Component::Container { components, .. } = &embed.components[0] else {
            panic!("updated container expected");
        };
        assert!(
            matches!(&components[..], [Component::Text(text)] if text == "Updated Reddit preview")
        );
    }
    assert!(message.components.is_empty());
}

#[test]
fn embed_galleries_are_consecutive_capped_and_suppressible() {
    let mut embeds = vec![
        json!({"url":"https://example.com/post","title":"gallery","image":{"url":"https://cdn.discordapp.com/first"}}),
    ];
    for index in 0..6 {
        embeds.push(json!({"url":"https://example.com/post","title":"","description":"","image":{"url":format!("https://cdn.discordapp.com/{index}")}}));
    }
    embeds.push(json!({"url":"https://example.com/other","image":{"url":"https://cdn.discordapp.com/other"}}));
    embeds.push(
        json!({"url":"https://example.com/post","image":{"url":"https://cdn.discordapp.com/last"}}),
    );
    let embeds: Vec<model::Embed> = model::parse_entries(json!(embeds));
    let mapped = mapping::map_embeds(&embeds, 0);
    assert_eq!(mapped.len(), 3);
    assert_eq!(mapped[0].images.len(), 4);
    assert_eq!(mapped[0].images[3].url, "https://cdn.discordapp.com/2");
    assert_eq!(mapped[2].images.len(), 1);
    assert!(mapping::map_embeds(&embeds, 1 << 2).is_empty());
}

#[test]
fn embed_merge_same_url_skips_duplicate_thumbnail() {
    let embeds: Vec<model::Embed> = model::parse_entries(json!([
        {
            "url": "https://fixupx.com/sample_account/status/1191169333657600100",
            "author": {"name": "Sample Author"},
            "description": "tweet text",
            "image": {"url": "https://cdn.discordapp.com/photo"}
        },
        {
            "url": "https://fixupx.com/sample_account/status/1191169333657600100",
            "thumbnail": {"url": "https://cdn.discordapp.com/photo"}
        }
    ]));
    let mapped = mapping::map_embeds(&embeds, 0);
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].images.len(), 1);
    assert_eq!(mapped[0].images[0].url, "https://cdn.discordapp.com/photo");
}

#[test]
fn embed_merge_same_url_adds_distinct_thumbnail() {
    let embeds: Vec<model::Embed> = model::parse_entries(json!([
        {
            "url": "https://fixupx.com/sample_account/status/1191169333657600100",
            "description": "tweet text",
            "image": {"url": "https://cdn.discordapp.com/first"}
        },
        {
            "url": "https://fixupx.com/sample_account/status/1191169333657600100",
            "thumbnail": {"url": "https://cdn.discordapp.com/second"}
        }
    ]));
    let mapped = mapping::map_embeds(&embeds, 0);
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].images.len(), 2);
    assert_eq!(mapped[0].images[0].url, "https://cdn.discordapp.com/first");
    assert_eq!(mapped[0].images[1].url, "https://cdn.discordapp.com/second");
}

#[test]
fn embed_merge_same_url_folds_four_embeds() {
    let embeds: Vec<model::Embed> = model::parse_entries(json!([
        {
            "url": "https://fixupx.com/sample_account/status/1",
            "title": "tweet",
            "image": {"url": "https://cdn.discordapp.com/0"}
        },
        {
            "url": "https://fixupx.com/sample_account/status/1",
            "color": 123,
            "footer": {"text": "first"},
            "image": {"url": "https://cdn.discordapp.com/1"}
        },
        {
            "url": "https://fixupx.com/sample_account/status/1",
            "color": 456,
            "footer": {"text": "second"},
            "image": {"url": "https://cdn.discordapp.com/2"}
        },
        {
            "url": "https://fixupx.com/sample_account/status/1",
            "color": 789,
            "footer": {"text": "third"},
            "image": {"url": "https://cdn.discordapp.com/3"}
        }
    ]));
    let mapped = mapping::map_embeds(&embeds, 0);
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].title.as_ref().unwrap().text, "tweet");
    assert!(mapped[0].color.is_none() && mapped[0].footer.is_none());
    assert_eq!(mapped[0].images.len(), 4);
    for (index, image) in mapped[0].images.iter().enumerate() {
        assert_eq!(image.url, format!("https://cdn.discordapp.com/{index}"));
    }
}

#[test]
fn embed_merge_different_urls_stay_separate() {
    let embeds: Vec<model::Embed> = model::parse_entries(json!([
        {
            "url": "https://example.com/one",
            "title": "one",
            "image": {"url": "https://cdn.discordapp.com/one"}
        },
        {
            "url": "https://example.com/two",
            "title": "two",
            "image": {"url": "https://cdn.discordapp.com/two"}
        }
    ]));
    let mapped = mapping::map_embeds(&embeds, 0);
    assert_eq!(mapped.len(), 2);
    assert_eq!(mapped[0].title.as_ref().unwrap().text, "one");
    assert_eq!(mapped[1].title.as_ref().unwrap().text, "two");
    assert_eq!(mapped[0].images.len(), 1);
    assert_eq!(mapped[1].images.len(), 1);
}

#[test]
fn embed_merge_same_url_video_follower_moves_video_to_first() {
    let embeds: Vec<model::Embed> = model::parse_entries(json!([
        {
            "url": "https://example.com/clip",
            "title": "clip",
            "image": {"url": "https://cdn.discordapp.com/still"}
        },
        {
            "url": "https://example.com/clip",
            "video": {"url": "https://media.discordapp.net/clip"}
        }
    ]));
    let mapped = mapping::map_embeds(&embeds, 0);
    assert_eq!(mapped.len(), 1);
    assert_eq!(mapped[0].images.len(), 1);
    assert_eq!(
        mapped[0].video.as_ref().map(|video| video.url.as_str()),
        Some("https://media.discordapp.net/clip")
    );
}

#[test]
fn remaining_components_map_with_defaults_and_unsafe_media_is_dropped() {
    let wire: Vec<model::Component> = model::parse_entries(json!([
        {"type":3}, {"type":5,"placeholder":"pick","disabled":true}, {"type":6}, {"type":7}, {"type":8},
        {"type":13,"file":{"url":"attachment://a.txt"},"name":"a.txt","size":12,"spoiler":true},
        {"type":14}, {"type":14,"divider":false,"spacing":2},
        {"type":11,"media":{"url":"https://evil.test/a"}},
        {"type":12,"items":[{"media":{"url":"https://evil.test/a"}}, {"media":{"url":"https://media.discordapp.net/a"},"description":"alt","spoiler":true}]}
    ]));
    let mapped = mapping::map_components(&wire);
    assert_eq!(mapped.len(), 9);
    assert!(matches!(
        &mapped[0],
        Component::Select {
            placeholder: None,
            disabled: false
        }
    ));
    assert!(
        matches!(&mapped[1], Component::Select { placeholder: Some(text), disabled: true } if text == "pick")
    );
    assert!(
        matches!(&mapped[5], Component::File { name, size: 12, spoiler: true } if name == "a.txt")
    );
    assert!(matches!(
        &mapped[6],
        Component::Separator {
            divider: true,
            large: false
        }
    ));
    assert!(matches!(
        &mapped[7],
        Component::Separator {
            divider: false,
            large: true
        }
    ));
    assert!(
        matches!(&mapped[8], Component::Gallery(items) if items.len() == 1 && items[0].spoiler && items[0].description.as_deref() == Some("alt"))
    );
}

#[test]
fn message_updates_preserve_absent_fields_and_restore_suppressed_embeds() {
    let mut store = Store::from_ready(ready());
    let mut source: model::Message =
        serde_json::from_str(include_str!("fixtures/components_v2.json")).unwrap();
    source.channel_id = 2002;
    source.id = 500;
    source.embeds = model::parse_entries(json!([{"title":"original"}]));
    source.message_reference = Some(serde_json::from_value(json!({"message_id":"400"})).unwrap());
    source.referenced_message = Some(Some(Box::new(
        serde_json::from_value(json!({
            "id":"400","author":{"id":"7"},"components":[{"type":10,"content":"reply"}]
        }))
        .unwrap(),
    )));
    store.set_messages(2002, vec![source]);
    let message = &store.guilds[0].channel(2002).unwrap().messages[0];
    assert!(matches!(
        &message.reply,
        Some(ReplyState::Loaded(Reply { embeds: true, .. }))
    ));
    for update in [
        json!({"flags":4}),
        json!({"content":"edited"}),
        json!({"embeds":[{"title":"replacement"}]}),
    ] {
        let mut value = update;
        value["id"] = json!("500");
        value["channel_id"] = json!("2002");
        assert_eq!(
            store.apply(Event::MessageUpdate(Box::new(
                serde_json::from_value(value).unwrap()
            ))),
            Change::MessageChanged {
                channel_id: 2002,
                index: 0
            }
        );
        let message = &store.guilds[0].channel(2002).unwrap().messages[0];
        assert!(message.embeds.is_empty());
        assert_eq!(message.reactions.len(), 5);
        assert_eq!(message.components.len(), 1);
    }
    let update = serde_json::from_value(json!({"id":"500","channel_id":"2002","flags":0,"components":[{"type":10,"content":"new"},{"type":99}]})).unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    let message = &store.guilds[0].channel(2002).unwrap().messages[0];
    assert_eq!(
        message.embeds[0].title.as_ref().unwrap().text,
        "replacement"
    );
    assert!(matches!(&message.components[..], [Component::Text(text)] if text == "new"));
    let update = serde_json::from_value(
        json!({"id":"500","channel_id":"2002","embeds":[],"components":[],"reactions":[]}),
    )
    .unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    let message = &store.guilds[0].channel(2002).unwrap().messages[0];
    assert!(
        message.embeds.is_empty() && message.components.is_empty() && message.reactions.is_empty()
    );
}

#[test]
fn receive_only_reaction_events_update_loaded_messages_in_order() {
    fn reaction(user: u64, emoji: serde_json::Value) -> model::ReactionEvent {
        serde_json::from_value(
            json!({"channel_id":"2002","message_id":"500","user_id":user,"emoji":emoji}),
        )
        .unwrap()
    }
    let mut store = Store::from_ready(ready());
    let me = store.current_user_id;
    let mut source = messages().remove(0);
    source.id = 500;
    source.channel_id = 2002;
    source.reactions.clear();
    assert_eq!(
        store.apply(Event::MessageReactionAdd(reaction(
            me,
            json!({"name":"one"})
        ))),
        Change::None
    );
    store.set_messages(2002, vec![source]);
    let changed = Change::MessageChanged {
        channel_id: 2002,
        index: 0,
    };
    assert_eq!(
        store.apply(Event::MessageReactionAdd(reaction(
            me,
            json!({"name":"one"})
        ))),
        changed
    );
    assert_eq!(
        store.apply(Event::MessageReactionAdd(reaction(
            8,
            json!({"name":"one"})
        ))),
        changed
    );
    assert_eq!(
        store.apply(Event::MessageReactionAdd(reaction(
            8,
            json!({"id":"99","name":"custom","animated":true})
        ))),
        changed
    );
    store.apply(Event::MessageReactionAdd(reaction(
        me,
        json!({"id":"99","name":"renamed"}),
    )));
    let reactions = &store.guilds[0].channel(2002).unwrap().messages[0].reactions;
    assert_eq!(reactions.len(), 2);
    assert_eq!(reactions[0].emoji.name, "one");
    assert_eq!(reactions[0].count, 2);
    assert!(reactions.iter().all(|r| r.me));
    assert!(reactions[1].emoji.animated);
    assert_eq!(reactions[1].count, 2);
    store.apply(Event::MessageReactionRemove(reaction(
        me,
        json!({"name":"one"}),
    )));
    store.apply(Event::MessageReactionRemove(reaction(
        8,
        json!({"id":"99","name":"ignored"}),
    )));
    let reactions = &store.guilds[0].channel(2002).unwrap().messages[0].reactions;
    assert!(!reactions[0].me);
    assert!(reactions[1].me);
    assert_eq!(reactions[0].count, 1);
    store.apply(Event::MessageReactionRemove(reaction(
        8,
        json!({"name":"one"}),
    )));
    store.apply(Event::MessageReactionRemove(reaction(
        8,
        json!({"name":"absent"}),
    )));
    let many = serde_json::from_value(json!({"channel_id":"2002","message_id":"500","reactions":[null,{"users":[me,8],"emoji":{"id":"99","name":"renamed"}},{"users":[7],"emoji":{"name":"new"}},{"users":[],"emoji":{"name":"empty"}}]})).unwrap();
    assert_eq!(store.apply(Event::MessageReactionAddMany(many)), changed);
    let reactions = &store.guilds[0].channel(2002).unwrap().messages[0].reactions;
    assert_eq!(reactions.len(), 2);
    assert_eq!(reactions[0].emoji.id, Some(99));
    // The bulk event includes our already-counted reaction; only user 8 is new.
    assert_eq!(reactions[0].count, 2);
    assert_eq!(reactions[1].emoji.name, "new");
    assert_eq!(
        store.apply(Event::MessageReactionRemoveEmoji(reaction(
            8,
            json!({"id":"99","name":"different"})
        ))),
        changed
    );
    assert_eq!(
        store.guilds[0].channel(2002).unwrap().messages[0]
            .reactions
            .len(),
        1
    );
    assert_eq!(
        store.apply(Event::MessageReactionRemoveAll(reaction(8, json!({})))),
        changed
    );
    assert!(
        store.guilds[0].channel(2002).unwrap().messages[0]
            .reactions
            .is_empty()
    );
    let mut absent = reaction(me, json!({"name":"one"}));
    absent.message_id = 501;
    assert_eq!(store.apply(Event::MessageReactionAdd(absent)), Change::None);
}

#[test]
fn rail_folders_ready_join_delete_and_settings_updates() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut payload: serde_json::Value =
        serde_json::from_str(include_str!("../discord/fixtures/ready.json")).unwrap();
    payload["user_settings_proto"] = settings_fixtures::fixture(true).into();
    let mut ready: model::Ready = serde_json::from_value(payload).unwrap();
    ready
        .guilds
        .push(serde_json::from_value(json!({"id":"300","name":"New","owner_id":"1"})).unwrap());
    let mut store = Store::from_ready(ready);
    assert_eq!(
        store.rail,
        vec![
            RailEntry::Guild(300),
            RailEntry::Guild(100),
            RailEntry::Folder(Folder {
                id: 42,
                name: Some("Gaming".into()),
                color: Some(0x5865F2),
                guild_ids: vec![200],
            })
        ]
    );
    assert_eq!(store.guild_by_id(100).unwrap().name, "First");
    let guild = serde_json::from_value(json!({"id":"400","name":"Joined","owner_id":"1"})).unwrap();
    store.apply(Event::GuildUpsert(Box::new(guild)));
    assert_eq!(store.rail[0], RailEntry::Guild(400));
    let before = format!("{:?}", store.rail);
    assert_eq!(
        store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
            settings: model::SettingsProto {
                kind: 1,
                proto: STANDARD.encode([8, 1])
            },
            partial: true,
        }))),
        Change::None
    );
    assert_eq!(format!("{:?}", store.rail), before);
    assert_eq!(
        store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
            settings: model::SettingsProto {
                kind: 1,
                proto: STANDARD.encode([114, 0])
            },
            partial: true,
        }))),
        Change::Structure
    );
    assert_eq!(
        store.rail,
        vec![
            RailEntry::Guild(400),
            RailEntry::Guild(200),
            RailEntry::Guild(100),
            RailEntry::Guild(300)
        ]
    );
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto {
            kind: 1,
            proto: settings_fixtures::fixture(false),
        },
        partial: false,
    })));
    store.apply(Event::GuildDelete { id: 200 });
    assert!(store.guild_by_id(200).is_none());
    assert_eq!(
        store.rail,
        vec![
            RailEntry::Guild(400),
            RailEntry::Guild(300),
            RailEntry::Guild(100)
        ]
    );
    let seed = Store::seed();
    assert!(
        matches!(&seed.rail[1], RailEntry::Folder(folder) if folder.name.as_deref() == Some("Gaming") && folder.guild_ids == [2, 3])
    );
}

#[test]
fn avatar_defaults_and_image_attachments() {
    let mut user = map_user(&ready().user);
    user.id = 5 << 22;
    for discriminator in [None, Some("0".to_owned())] {
        user.discriminator = discriminator;
        assert!(matches!(
            user.avatar_spec(),
            ImageSpec::DefaultAvatar { index: 5 }
        ));
    }
    user.discriminator = Some("1234".into());
    assert!(matches!(
        user.avatar_spec(),
        ImageSpec::DefaultAvatar { index: 4 }
    ));
    user.avatar = Some("a_hash".into());
    assert!(matches!(user.avatar_spec(), ImageSpec::Avatar { .. }));
    let mut store = Store::from_ready(ready());
    assert!(matches!(
        store.guilds[0].icon_spec(),
        Some(ImageSpec::GuildIcon { guild_id: 200, .. })
    ));
    store.set_messages(2002, messages());
    let update = serde_json::from_value(json!({"id":"62","channel_id":"2002","attachments":[{
        "id":"77","filename":"image.GIF","url":"https://cdn.discordapp.com/attachments/1/2/image.gif",
        "proxy_url":"https://media.discordapp.net/attachments/1/2/image.gif", "width":400,"height":200,"size":42
    }, {"id":"bad"}]})).unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    let message = &store.guilds[0].channel(2002).unwrap().messages[1];
    assert_eq!(message.attachments.len(), 1);
    assert!(matches!(
        message.attachments[0].image_spec(),
        Some(ImageSpec::Attachment {
            id: 77,
            width: 400,
            height: 200,
            ..
        })
    ));
    let mut attachment = message.attachments[0].clone();
    attachment.width = None;
    assert!(attachment.image_spec().is_none());
    attachment.width = Some(400);
    attachment.filename = "not-image.pdf".into();
    assert!(attachment.image_spec().is_none());
    attachment.content_type = Some("image/png".into());
    assert!(attachment.image_spec().is_some());
    let update =
        serde_json::from_value(json!({"id":"62","channel_id":"2002","attachments":[]})).unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    assert!(
        store.guilds[0].channel(2002).unwrap().messages[1]
            .attachments
            .is_empty()
    );
}

#[test]
fn voice_channels_follow_text_channels_within_a_category() {
    let mut payload: serde_json::Value =
        serde_json::from_str(include_str!("../discord/fixtures/ready.json")).unwrap();
    let guild = &mut payload["guilds"][0];
    let owner = guild["properties"]["owner_id"].clone();
    guild["channels"] = json!([
        {"id": "900", "type": 4, "name": "Category", "position": 0},
        {"id": "901", "type": 2, "name": "voice-first", "position": 0, "parent_id": "900"},
        {"id": "902", "type": 0, "name": "text-second", "position": 1, "parent_id": "900"},
        {"id": "903", "type": 13, "name": "stage", "position": 2, "parent_id": "900"},
        {"id": "904", "type": 5, "name": "news-last", "position": 3, "parent_id": "900"},
        {"id": "905", "type": 15, "name": "forum", "position": 0, "parent_id": "900"},
        {"id": "906", "type": 16, "name": "media-last", "position": 4, "parent_id": "900"},
    ]);
    payload["user"]["id"] = owner;
    let ready: model::Ready = serde_json::from_value(payload).unwrap();
    let guild_id = ready.guilds[0].id;
    let store = Store::from_ready(ready);
    let guild = store.guilds.iter().find(|g| g.id == guild_id).unwrap();
    let names: Vec<_> = guild
        .categories
        .iter()
        .flat_map(|category| category.channels.iter().map(|c| c.name.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            "forum",
            "text-second",
            "news-last",
            "media-last",
            "voice-first",
            "stage"
        ]
    );
    assert!(guild.channel(905).unwrap().kind.is_forum());
    assert!(guild.channel(906).unwrap().kind.is_forum());
}

#[test]
fn guild_banner_comes_from_ready_and_updates_and_clears() {
    let mut payload: serde_json::Value =
        serde_json::from_str(include_str!("../discord/fixtures/ready.json")).unwrap();
    payload["guilds"][0]["properties"]["banner"] = json!("a_ready_banner");
    let ready: model::Ready = serde_json::from_value(payload).unwrap();
    let guild_id = ready.guilds[0].id;
    let mut store = Store::from_ready(ready);
    assert_eq!(
        store.guild_banner(guild_id),
        Some(ImageSpec::GuildBanner {
            guild_id,
            hash: "a_ready_banner".into(),
        })
    );

    let update: model::Guild =
        serde_json::from_value(json!({"id": guild_id.to_string(), "banner": "updated"})).unwrap();
    assert_eq!(
        store.apply(Event::GuildUpsert(Box::new(update))),
        Change::Structure
    );
    assert_eq!(
        store.guild_banner(guild_id),
        Some(ImageSpec::GuildBanner {
            guild_id,
            hash: "updated".into(),
        })
    );

    let clear: model::Guild =
        serde_json::from_value(json!({"id": guild_id.to_string(), "banner": null})).unwrap();
    store.apply(Event::GuildUpsert(Box::new(clear)));
    assert_eq!(store.guild_banner(guild_id), None);
}

#[test]
fn ready_mapping_order_permissions_and_badges() {
    let ready = ready();
    assert_eq!(
        ready.guilds[1].members.as_ref().unwrap()[0].roles,
        Vec::<u64>::new()
    );
    let mut store = Store::seed();
    assert_eq!(
        store.apply(Event::Ready(Box::new(ready))),
        Change::Structure
    );
    assert_eq!(
        store.guilds.iter().map(|g| g.id).collect::<Vec<_>>(),
        [200, 100]
    );
    let guild = &store.guilds[0];
    assert_eq!(
        guild
            .categories
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["", "Earlier", "Later"]
    );
    assert_eq!(
        guild.categories[1]
            .channels
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        [2003, 2002]
    );
    assert!(guild.channel(2001).is_none());
    assert!(guild.channel(2002).unwrap().unread);
    assert!(!guild.channel(2003).unwrap().unread);
    assert!(!guild.channel(2004).unwrap().unread);
    assert!(guild.channel(2007).is_some());
    assert!(guild.channel(2007).unwrap().kind.is_forum());
    assert!(guild.channel(2008).is_none());
    assert_eq!(guild.channel(2002).unwrap().mentions, 2);
    assert_eq!(guild.channel(2003).unwrap().mentions, 1);
    assert_eq!(guild.channel(2004).unwrap().mentions, 0);
    assert!(guild.channel(2006).unwrap().kind == ChannelKind::Announcement);
    assert_eq!(guild.mentions, 3);
    assert_eq!(
        guild
            .channels()
            .map(|channel| channel.mentions)
            .sum::<u32>(),
        guild.mentions
    );
    assert!(guild.unread);
    assert_eq!(store.dm_mentions, 4);
    assert_eq!(guild.initials, "SGN");
    assert_eq!(guild.icon.as_deref(), Some("guild-hash"));
    assert!(store.guilds[1].channel(1001).is_some());
    assert_eq!(store.user(2).unwrap().name, "A Writer");
    assert_eq!(
        store.user(2).unwrap().avatar.as_deref(),
        Some("avatar-hash")
    );
}

#[test]
fn mentioning_message_increments_channel_and_guild_mentions() {
    let mut store = Store::from_ready(ready());
    let message: model::Message = serde_json::from_value(json!({
        "id":"100", "channel_id":"2004", "guild_id":"200",
        "author":{"id":"2","username":"writer"},
        "timestamp":"2026-10-22T12:00:00Z", "content":"ping",
        "mentions":[{"id":"1"}]
    }))
    .unwrap();
    store.apply(Event::MessageCreate(Box::new(message)));
    let guild = &store.guilds[0];
    assert_eq!(guild.channel(2004).unwrap().mentions, 1);
    assert_eq!(guild.mentions, 4);
    assert_eq!(
        guild
            .channels()
            .map(|channel| channel.mentions)
            .sum::<u32>(),
        guild.mentions
    );
}

#[test]
fn own_message_does_not_increment_channel_mentions() {
    let mut store = Store::from_ready(ready());
    // Older than the channel's read state so the own-message ack cannot mask the count.
    let message: model::Message = serde_json::from_value(json!({
        "id":"45", "channel_id":"2002", "guild_id":"200",
        "author":{"id":"1","username":"owner"},
        "timestamp":"2026-10-22T12:00:00Z", "content":"self ping",
        "mentions":[{"id":"1"}]
    }))
    .unwrap();
    store.apply(Event::MessageCreate(Box::new(message)));
    assert_eq!(store.guilds[0].channel(2002).unwrap().mentions, 2);
    assert_eq!(store.guilds[0].mentions, 3);
}

#[test]
fn ack_resets_channel_and_guild_mentions() {
    let mut store = Store::from_ready(ready());
    store.apply(Event::MessageAck {
        channel_id: 2002,
        message_id: 60,
    });
    let guild = &store.guilds[0];
    assert_eq!(guild.channel(2002).unwrap().mentions, 0);
    assert_eq!(guild.channel(2003).unwrap().mentions, 1);
    assert_eq!(guild.mentions, 1);
}

#[test]
fn structure_rebuild_preserves_channel_mentions() {
    let mut store = Store::from_ready(ready());
    let update = serde_json::from_value(json!({
        "id":"2002", "guild_id":"200", "name":"renamed"
    }))
    .unwrap();
    store.apply(Event::ChannelUpsert(Box::new(update)));
    let guild = &store.guilds[0];
    assert_eq!(guild.channel(2002).unwrap().name, "renamed");
    assert_eq!(guild.channel(2002).unwrap().mentions, 2);
    assert_eq!(guild.channel(2003).unwrap().mentions, 1);
    assert_eq!(guild.mentions, 3);
}

#[test]
fn forum_channels_map_tags_posts_and_new_post_counts() {
    let threads = vec![
        forum_thread(
            3001,
            2007,
            "First post",
            Some(3001),
            "2026-10-20T12:00:00Z",
            0,
            2,
        ),
        forum_thread(
            3002,
            2007,
            "Second post",
            Some(3002),
            "2026-10-21T12:00:00Z",
            0,
            4,
        ),
    ];
    let store = Store::from_ready(forum_ready(threads, Some(3000), None));
    let guild = store.guild_by_id(200).unwrap();
    let forum = guild.channel(2007).unwrap();
    assert!(forum.kind.is_forum());
    assert_eq!(forum.new_posts, 2);
    assert!(forum.unread);
    assert!(guild.channel(2009).unwrap().kind.is_forum());

    let tags = store.forum_tags(2007);
    assert_eq!(tags.len(), 2);
    assert_eq!(tags[0].name, "Help");
    assert_eq!(
        tags[0].emoji,
        Some(StatusEmoji::Unicode("\u{1F9E0}".into()))
    );
    assert!(tags[1].moderated);
    assert!(matches!(
        tags[1].emoji,
        Some(StatusEmoji::Custom { id: 555, .. })
    ));

    let posts = store.forum_posts(2007);
    assert_eq!(
        posts
            .iter()
            .map(|post| post.name.as_str())
            .collect::<Vec<_>>(),
        ["Second post", "First post"]
    );
    assert!(guild.channel(3001).is_some());
    assert!(store.can_send(3001));
    assert!(!store.can_send(2007));
}

#[test]
fn forum_new_posts_track_thread_create_and_ack() {
    let threads = vec![forum_thread(
        3001,
        2007,
        "Existing",
        Some(3001),
        "2026-10-20T12:00:00Z",
        0,
        1,
    )];
    let mut store = Store::from_ready(forum_ready(threads, Some(3000), None));
    assert_eq!(
        store
            .guild_by_id(200)
            .unwrap()
            .channel(2007)
            .unwrap()
            .new_posts,
        1
    );

    let thread: model::Channel = serde_json::from_value(forum_thread(
        3002,
        2007,
        "Live post",
        Some(3002),
        "2026-10-22T12:00:00Z",
        0,
        0,
    ))
    .unwrap();
    store.apply(Event::ChannelUpsert(Box::new(thread)));
    let forum = store.guild_by_id(200).unwrap().channel(2007).unwrap();
    assert_eq!(forum.new_posts, 2);
    assert!(forum.unread);

    store.apply(Event::MessageAck {
        channel_id: 2007,
        message_id: 3002,
    });
    let forum = store.guild_by_id(200).unwrap().channel(2007).unwrap();
    assert_eq!(forum.new_posts, 0);
    assert!(!forum.unread);
}

#[test]
fn forum_posts_order_by_pinned_activity_and_creation() {
    let threads = vec![
        forum_thread(
            3001,
            2007,
            "older",
            Some(1_200_000_000_000_000_000),
            "2026-10-20T12:00:00Z",
            0,
            3,
        ),
        forum_thread(
            3002,
            2007,
            "pinned",
            Some(1_100_000_000_000_000_000),
            "2026-10-21T12:00:00Z",
            2,
            2,
        ),
        forum_thread(
            3003,
            2007,
            "newest",
            Some(1_000_000_000_000_000_000),
            "2026-10-22T12:00:00Z",
            0,
            1,
        ),
    ];
    let store = Store::from_ready(forum_ready(threads.clone(), Some(3000), None));
    assert_eq!(
        store
            .forum_posts(2007)
            .iter()
            .map(|post| post.name.as_str())
            .collect::<Vec<_>>(),
        ["pinned", "older", "newest"]
    );

    let store = Store::from_ready(forum_ready(threads, Some(3000), Some(1)));
    assert_eq!(
        store
            .forum_posts(2007)
            .iter()
            .map(|post| post.name.as_str())
            .collect::<Vec<_>>(),
        ["pinned", "newest", "older"]
    );
}

#[test]
fn forum_posts_merge_ready_search_and_thread_events() {
    let threads = vec![forum_thread(
        3001,
        2007,
        "Ready post",
        Some(3001),
        "2026-10-20T12:00:00Z",
        0,
        1,
    )];
    let mut store = Store::from_ready(forum_ready(threads, Some(3000), None));
    let search: model::ForumSearch = serde_json::from_value(json!({
        "threads": [forum_thread(
            3002, 2007, "Search post", Some(3400), "2026-10-21T12:00:00Z", 0, 5
        )],
        "first_messages": [{
            "id": "3401",
            "channel_id": "3002",
            "author": {"id": "2", "username": "writer"},
            "timestamp": "2026-10-21T12:00:00Z",
            "content": "first **body**",
            "attachments": [{
                "id": "77",
                "filename": "image.png",
                "url": "https://cdn.discordapp.com/attachments/1/77/image.png",
                "proxy_url": "https://media.discordapp.net/attachments/1/77/image.png",
                "width": 400,
                "height": 200,
                "content_type": "image/png"
            }]
        }],
        "has_more": true
    }))
    .unwrap();
    store.apply_forum_search(2007, search);
    let post = store
        .forum_posts(2007)
        .into_iter()
        .find(|post| post.id == 3002)
        .unwrap();
    assert_eq!(post.name, "Search post");
    assert_eq!(post.preview.as_deref(), Some("first **body**"));
    assert!(matches!(
        post.thumbnail,
        Some(ImageSpec::Attachment {
            id: 77,
            width: 400,
            height: 200,
            ..
        })
    ));

    let search: model::ForumSearch = serde_json::from_value(json!({
        "threads": [forum_thread(
            3002, 2007, "Renamed", Some(3400), "2026-10-21T12:00:00Z", 0, 5
        )],
        "has_more": false
    }))
    .unwrap();
    store.apply_forum_search(2007, search);
    let posts = store.forum_posts(2007);
    assert_eq!(posts.len(), 2);
    assert_eq!(
        posts
            .iter()
            .find(|post| post.id == 3002)
            .unwrap()
            .preview
            .as_deref(),
        Some("first **body**")
    );

    let update: model::Channel = serde_json::from_value(json!({
        "id": "3002", "type": 11, "guild_id": "200", "parent_id": "2007",
        "name": "Edited", "flags": 2, "message_count": 9, "applied_tags": ["4002"],
        "last_message_id": "3500",
        "thread_metadata": {
            "archived": true, "locked": true,
            "create_timestamp": "2026-10-21T12:00:00Z"
        }
    }))
    .unwrap();
    store.apply(Event::ChannelUpsert(Box::new(update)));
    let post = store
        .forum_posts(2007)
        .into_iter()
        .find(|post| post.id == 3002)
        .unwrap();
    assert_eq!(post.name, "Edited");
    assert!(post.pinned && post.locked && post.archived);
    assert_eq!(post.tag_ids, [4002]);
    assert_eq!(post.message_count, 9);

    let sync: model::ThreadListSync = serde_json::from_value(json!({
        "guild_id": "200",
        "threads": [forum_thread(
            3003, 2007, "Synced", Some(3003), "2026-10-22T12:00:00Z", 0, 0
        )]
    }))
    .unwrap();
    store.apply(Event::ThreadListSync(Box::new(sync)));
    assert!(store.forum_posts(2007).iter().any(|post| post.id == 3003));

    store.apply(Event::ChannelDelete { id: 3002 });
    assert!(!store.forum_posts(2007).iter().any(|post| post.id == 3002));
}

#[test]
fn thread_message_create_bumps_forum_post_and_thread_channel_loads() {
    let threads = vec![forum_thread(
        3001,
        2007,
        "Body",
        Some(3001),
        "2026-10-20T12:00:00Z",
        0,
        2,
    )];
    let mut ready = forum_ready(threads, Some(3000), None);
    ready.read_state.entries.push(model::ReadState {
        id: 3001,
        last_message_id: Some(3000),
        mention_count: 1,
        kind: 0,
    });
    let mut store = Store::from_ready(ready);
    let thread = store.guild_by_id(200).unwrap().channel(3001).unwrap();
    assert!(thread.unread);
    assert_eq!(thread.mentions, 1);
    let message: model::Message = serde_json::from_value(json!({
        "id": "3100",
        "channel_id": "3001",
        "guild_id": "200",
        "author": {"id": "2", "username": "writer"},
        "timestamp": "2026-10-22T12:00:00Z",
        "content": "the post body",
        "attachments": [{
            "id": "88",
            "filename": "photo.jpg",
            "url": "https://cdn.discordapp.com/attachments/1/88/photo.jpg",
            "proxy_url": "https://media.discordapp.net/attachments/1/88/photo.jpg",
            "width": 300,
            "height": 500,
            "content_type": "image/jpeg"
        }]
    }))
    .unwrap();
    store.apply(Event::MessageCreate(Box::new(message.clone())));
    let post = store
        .forum_posts(2007)
        .into_iter()
        .find(|post| post.id == 3001)
        .unwrap();
    assert_eq!(post.message_count, 3);
    assert_eq!(
        post.last_activity,
        local_timestamp("2026-10-22T12:00:00Z").unwrap()
    );
    assert_eq!(post.preview.as_deref(), Some("the post body"));
    assert!(matches!(
        post.thumbnail,
        Some(ImageSpec::Attachment { id: 88, .. })
    ));
    assert!(post.unread);
    assert!(
        store
            .guild_by_id(200)
            .unwrap()
            .channel(3001)
            .unwrap()
            .unread
    );

    store.set_messages(3001, vec![message]);
    let thread = store.guild_by_id(200).unwrap().channel(3001).unwrap();
    assert!(thread.messages_loaded);
    assert_eq!(thread.messages.len(), 1);
    assert!(store.at_present(3001));
    assert!(store.can_send(3001));

    store.apply(Event::MessageAck {
        channel_id: 3001,
        message_id: 3100,
    });
    let thread = store.guild_by_id(200).unwrap().channel(3001).unwrap();
    assert!(!thread.unread);
    assert_eq!(thread.mentions, 0);
    assert!(!store.forum_posts(2007)[0].unread);
}

#[test]
fn message_history_order_local_time_and_embeds() {
    let mut store = Store::from_ready(ready());
    assert!(!store.guilds[0].channel(2002).unwrap().messages_loaded);
    store.set_messages(2002, messages());
    let channel = store.guilds[0].channel(2002).unwrap();
    assert!(channel.messages_loaded);
    assert_eq!(
        channel.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
        [61, 62]
    );
    let timestamp = channel.messages[1].timestamp;
    let expected = chrono::DateTime::parse_from_rfc3339("2026-10-22T12:34:56.123+00:00")
        .unwrap()
        .with_timezone(&chrono::Local);
    assert_eq!(
        (
            timestamp.year,
            timestamp.month,
            timestamp.day,
            timestamp.hour,
            timestamp.minute
        ),
        (
            expected.year() as u16,
            expected.month() as u8,
            expected.day() as u8,
            expected.hour() as u8,
            expected.minute() as u8
        )
    );
    let embed = &channel.messages[1].embeds[0];
    assert!(embed.title.is_none());
    assert_eq!(embed.author.as_ref().unwrap().text, "Status");
    assert_eq!(channel.messages[1].embeds.len(), 2);
    assert_eq!(embed.description.as_deref(), Some("one\ntwo"));
    assert_eq!(embed.fields[0].name, "Count");
    assert_eq!(embed.fields[0].value, "3");
    assert_eq!(embed.footer.as_ref().unwrap().text, "footer");
    assert_eq!(embed.color, None);
    assert_eq!(channel.messages[1].content, "<@1> **raw**");
    assert!(store.user(3).unwrap().bot);
}

#[test]
fn apply_messages_loaded_unloaded_update_delete_and_ack() {
    let mut store = Store::from_ready(ready());
    let mut message = messages().remove(0);
    message.channel_id = 2004;
    message.id = 100;
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(message.clone()))),
        Change::Structure
    );
    assert!(store.guilds[0].channel(2004).unwrap().messages.is_empty());
    assert!(store.guilds[0].channel(2004).unwrap().unread);
    store.set_messages(2004, vec![]);
    message.id = 101;
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(message.clone()))),
        Change::MessageInserted {
            channel_id: 2004,
            index: 0
        }
    );
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(message))),
        Change::None
    );
    let update =
        serde_json::from_value(json!({"id":"101", "channel_id":"2004", "content":"", "embeds":[]}))
            .unwrap();
    assert_eq!(
        store.apply(Event::MessageUpdate(Box::new(update))),
        Change::MessageChanged {
            channel_id: 2004,
            index: 0
        }
    );
    let updated = &store.guilds[0].channel(2004).unwrap().messages[0];
    assert!(updated.content.is_empty());
    assert!(updated.embeds.is_empty());
    assert_eq!(updated.author_id, 3);
    let update = serde_json::from_value(json!({"id":"101", "channel_id":"2004"})).unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    assert!(
        store.guilds[0].channel(2004).unwrap().messages[0]
            .content
            .is_empty()
    );
    store.apply(Event::MessageAck {
        channel_id: 2004,
        message_id: 100,
    });
    assert!(store.guilds[0].channel(2004).unwrap().unread);
    store.apply(Event::MessageAck {
        channel_id: 2004,
        message_id: 101,
    });
    assert!(!store.guilds[0].channel(2004).unwrap().unread);
    assert_eq!(
        store.apply(Event::MessageDelete {
            channel_id: 2004,
            id: 101
        }),
        Change::MessageRemoved {
            channel_id: 2004,
            index: 0
        }
    );
    assert_eq!(
        store.apply(Event::MessageDelete {
            channel_id: 2004,
            id: 101
        }),
        Change::None
    );
}

#[test]
fn partial_structure_updates_preserve_history_and_permissions() {
    let ready = ready();
    let source_users = ready.users.as_ptr();
    let mut store = Store::from_ready(ready);
    assert_eq!(store.source.as_ref().unwrap().users.as_ptr(), source_users);
    let other_guild_name = store.guilds[1].name.as_ptr();
    store.set_messages(2002, messages());
    let update =
        serde_json::from_value(json!({"id":"2002", "guild_id":"200", "name":"renamed"})).unwrap();
    store.apply(Event::ChannelUpsert(Box::new(update)));
    assert_eq!(store.source.as_ref().unwrap().users.as_ptr(), source_users);
    assert_eq!(store.guilds[1].name.as_ptr(), other_guild_name);
    assert_eq!(store.guilds[0].channel(2002).unwrap().name, "renamed");
    assert_eq!(store.guilds[0].channel(2002).unwrap().messages.len(), 2);
    let guild = serde_json::from_value(json!({"id":"200", "name":"Changed", "icon":null})).unwrap();
    store.apply(Event::GuildUpsert(Box::new(guild)));
    assert_eq!(store.guilds[0].name, "Changed");
    assert!(store.guilds[0].icon.is_none());
    assert!(store.guilds[0].channel(2001).is_none());
    let hide = serde_json::from_value(json!({"id":"2002", "permission_overwrites":[{"id":"1", "type":1, "allow":"0", "deny":"1024"}]})).unwrap();
    store.apply(Event::ChannelUpsert(Box::new(hide)));
    assert!(store.guilds[0].channel(2002).is_none());
}

#[test]
fn badge_deltas_and_indexes_survive_structure_changes() {
    let mut store = Store::from_ready(ready());
    store.apply(Event::MessageAck {
        channel_id: 2002,
        message_id: 60,
    });
    assert_eq!(store.guilds[0].mentions, 1);
    assert!(!store.guilds[0].unread);
    store.apply(Event::MessageAck {
        channel_id: 900,
        message_id: 1,
    });
    assert_eq!(store.dm_mentions, 0);
    store.apply(Event::ChannelDelete { id: 2003 });
    assert_eq!(store.guilds[0].mentions, 0);
    let mut message = messages().remove(0);
    message.id = 100;
    message.channel_id = 2004;
    store.apply(Event::MessageCreate(Box::new(message.clone())));
    assert!(store.guilds[0].unread);
    store.apply(Event::MessageAck {
        channel_id: 2004,
        message_id: 100,
    });
    assert!(!store.guilds[0].unread);
    store.apply(Event::GuildDelete { id: 200 });
    assert_eq!(store.guilds[0].id, 100);
    message.id = 101;
    message.channel_id = 1001;
    store.apply(Event::MessageCreate(Box::new(message)));
    assert!(store.guilds[0].channel(1001).unwrap().unread);
    assert!(store.guilds[0].unread);
}

#[test]
fn thousand_message_creates_with_large_ready_are_fast() {
    let mut ready = ready();
    ready.guilds = (0..100u64)
        .map(|index| {
            let id = 10_000 + index;
            let channels: Vec<_> = (0..20u64)
                .map(|channel| {
                    json!({
                        "id": (100_000 + index * 20 + channel).to_string(), "type": 0,
                        "name": "busy", "last_message_id": "5000"
                    })
                })
                .collect();
            serde_json::from_value(json!({
                "id": id.to_string(), "properties": {"name": "Busy Guild", "owner_id": "99"},
                "roles": [{"id": id.to_string(), "permissions": "1024"}],
                "members": [{"user_id": "1", "roles": []}], "channels": channels
            }))
            .unwrap()
        })
        .collect();
    ready.read_state.entries = (0..2000)
        .map(|index| model::ReadState {
            id: 100_000 + index,
            last_message_id: Some(5000),
            mention_count: 0,
            kind: 0,
        })
        .collect();
    ready.users = (0..20_000)
        .map(|index| model::User {
            id: 10 + index,
            username: format!("user-{index}"),
            ..Default::default()
        })
        .collect();
    ready.private_channels.clear();
    let mut store = Store::from_ready(ready);
    let template = messages().remove(0);
    let events: Vec<_> = (0..1000)
        .map(|index| {
            let mut message = template.clone();
            message.id = 6000 + index;
            message.channel_id = 100_000 + index * 2;
            message.author.id = 20_009 - index;
            store.set_messages(message.channel_id, Vec::new());
            Event::MessageCreate(Box::new(message))
        })
        .collect();
    let start = std::time::Instant::now();
    for event in events {
        assert!(matches!(
            store.apply(event),
            Change::MessageInserted { index: 0, .. }
        ));
    }
    let elapsed = start.elapsed();
    println!(
        "1000 MESSAGE_CREATE events / 2000 channels / 2000 read states / 20000 users: {elapsed:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(250),
        "message apply took {elapsed:?}"
    );
    assert_eq!(
        store
            .guilds
            .iter()
            .flat_map(|g| g.channels())
            .filter(|c| c.unread)
            .count(),
        1000
    );
    assert!(store.guilds.iter().all(|g| g.unread));
}

fn social_fixture(key: &str) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(include_str!("../discord/fixtures/social.json"))
        .unwrap()[key]
        .clone()
}

fn member_event(store: &mut Store, value: serde_json::Value) -> Change {
    store.apply(Event::MemberUpsert(Box::new(
        serde_json::from_value(value).unwrap(),
    )))
}

fn role_event(store: &mut Store, value: serde_json::Value) -> Change {
    store.apply(Event::RoleUpsert(Box::new(
        serde_json::from_value(value).unwrap(),
    )))
}

#[test]
fn supplemental_chunk_names_colors_avatars_and_missing_members() {
    let mut store = Store::from_ready(ready());
    assert!(store.user(1).unwrap().status == Status::Online);
    assert_eq!(store.member_name(200, 2), Some("A Writer"));
    assert_eq!(store.member_name(200, 77), None);
    assert_eq!(store.missing_members(200, [1, 2, 2, 3, 4]), [2, 3, 4]);
    assert!(store.missing_members(200, [2, 3, 4]).is_empty());
    let supplemental = serde_json::from_value(social_fixture("supplemental")).unwrap();
    assert_eq!(
        store.apply(Event::ReadySupplemental(Box::new(supplemental))),
        Change::Members { guild_id: None }
    );
    assert_eq!(store.member_name(200, 2), Some("Writer Nick"));
    assert_eq!(store.member_name(100, 2), Some("Other Nick"));
    assert!(store.user(2).unwrap().status == Status::Idle);
    assert_eq!(store.user(2).unwrap().activity.as_deref(), Some("Example Game"));
    assert!(store.user(2).unwrap().playing);
    let chunk = serde_json::from_value(social_fixture("chunk")).unwrap();
    assert_eq!(
        store.apply(Event::MembersChunk(Box::new(chunk))),
        Change::Members {
            guild_id: Some(200)
        }
    );
    assert_eq!(store.member_name(200, 3), Some("Three"));
    assert!(
        matches!(store.member_avatar_spec(200,3), Some(ImageSpec::GuildAvatar { guild_id:200,user_id:3,hash }) if hash == "guild_hash")
    );
    assert!(store.user(3).unwrap().avatar.is_none());
    assert!(store.user(3).unwrap().status == Status::DoNotDisturb);
    assert_eq!(
        store.user(3).unwrap().activity.as_deref(),
        Some("Streaming Making things")
    );
    assert!(store.missing_members(200, [3, 4]).is_empty());
    for (id, position, color, primary) in [
        (201, 1, 0x112233, 0),
        (202, 3, 0x445566, 0x778899),
        (203, 9, 0, 0),
    ] {
        assert_eq!(
            role_event(
                &mut store,
                json!({"guild_id":"200","role":{"id":id.to_string(),"name":format!("Role {id}"),"permissions":"0","position":position,"color":color,"colors":{"primary_color":primary},"hoist":true}})
            ),
            Change::Structure
        );
    }
    assert_eq!(
        member_event(
            &mut store,
            json!({"guild_id":"200","user":{"id":"2"},"roles":["201","202","203"]})
        ),
        Change::Members {
            guild_id: Some(200)
        }
    );
    assert_eq!(store.member_color(200, 2), Some(0x778899));
    assert_eq!(store.role(200, 202).unwrap().position, 3);
    assert!(store.role(200, 202).unwrap().hoist);
    assert_eq!(store.member_name(200, 2), Some("Writer Nick"));
    member_event(
        &mut store,
        json!({"guild_id":"200","user":{"id":"2"},"nick":null}),
    );
    assert_eq!(store.member_name(200, 2), Some("A Writer"));
    member_event(
        &mut store,
        json!({"guild_id":"200","user":{"id":"8","username":"username"},"roles":[]}),
    );
    assert_eq!(store.member_name(200, 8), Some("username"));
    assert_eq!(
        store.apply(Event::RoleDelete {
            guild_id: 200,
            role_id: 202
        }),
        Change::Structure
    );
    assert_eq!(store.member_color(200, 2), Some(0x112233));
    assert!(store.role(200, 202).is_none());
    store.apply(Event::Ready(Box::new(ready())));
    assert_eq!(store.missing_members(200, [4]), [4]);
}

#[test]
fn own_roles_recompute_permissions_and_member_removal() {
    let mut store = Store::from_ready(ready());
    assert!(store.guild_by_id(200).unwrap().channel(2002).is_some());
    assert_eq!(
        member_event(
            &mut store,
            json!({"guild_id":"200","user":{"id":"1"},"roles":[]})
        ),
        Change::Structure
    );
    assert!(store.guild_by_id(200).unwrap().channel(2002).is_none());
    assert!(
        store.source.as_ref().unwrap().guilds[0]
            .members
            .as_ref()
            .unwrap()[0]
            .roles
            .is_empty()
    );
    assert_eq!(
        member_event(
            &mut store,
            json!({"guild_id":"200","user":{"id":"1"},"roles":["201"]})
        ),
        Change::Structure
    );
    assert!(store.guild_by_id(200).unwrap().channel(2002).is_some());
    assert_eq!(
        store.apply(Event::MemberRemove(Box::new(
            serde_json::from_value(json!({"guild_id":"200","user":{"id":"1"}})).unwrap()
        ))),
        Change::Structure
    );
    assert!(store.guild_by_id(200).unwrap().channel(2002).is_none());
}

#[test]
fn member_list_all_ops_switching_and_item_presence() {
    let mut store = Store::from_ready(ready());
    role_event(
        &mut store,
        json!({"guild_id":"200","role":{"id":"201","name":"Mods","color":42}}),
    );
    let list: model::MemberListUpdate = serde_json::from_value(social_fixture("list")).unwrap();
    assert_eq!(
        store.apply(Event::MemberListUpdate(Box::new(list))),
        Change::Members {
            guild_id: Some(200)
        }
    );
    let rows = &store.member_list(200).unwrap().rows;
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows[0],
        Some(MemberRow::Group {
            name: "Mods".into(),
            count: 1
        })
    );
    assert_eq!(rows[1], Some(MemberRow::Member { user_id: 2 }));
    assert_eq!(store.member_name(200, 2), Some("List Nick"));
    role_event(
        &mut store,
        json!({"guild_id":"200","role":{"id":"201","name":"Renamed Mods","color":43}}),
    );
    assert_eq!(
        store.member_list(200).unwrap().rows[0],
        Some(MemberRow::Group {
            name: "Renamed Mods".into(),
            count: 1
        })
    );
    assert_eq!(
        store.user(2).unwrap().activity.as_deref(),
        Some("\u{1f3ae} grinding")
    );
    let mut update = social_fixture("list");
    update["ops"] = json!([
        {"op":"INSERT","index":1,"item":{"member":{"user":{"id":"8","username":"inserted"},"roles":[]}}},
        {"op":"UPDATE","index":2,"item":{"member":{"user":{"id":"9","username":"updated"},"roles":[]}}},
        {"op":"DELETE","index":3},
        {"op":"INVALIDATE","range":[0,0]}
    ]);
    store.apply(Event::MemberListUpdate(Box::new(
        serde_json::from_value(update.clone()).unwrap(),
    )));
    assert_eq!(
        store.member_list(200).unwrap().rows,
        vec![
            None,
            Some(MemberRow::Member { user_id: 8 }),
            Some(MemberRow::Member { user_id: 9 }),
            Some(MemberRow::Member { user_id: 3 })
        ]
    );
    update["id"] = "different".into();
    update["ops"] = json!([{"op":"UPDATE","index":1,"item":{"member":{"user":{"id":"77","username":"ignored"}}}}]);
    store.apply(Event::MemberListUpdate(Box::new(
        serde_json::from_value(update.clone()).unwrap(),
    )));
    assert_eq!(store.member_list(200).unwrap().id, "everyone");
    assert!(store.user(77).is_none());
    update["ops"] = json!([{"op":"SYNC","range":[2,3],"items":[{"group":{"id":"online","count":1}},{"member":{"user":{"id":"77","username":"synced"}}}]}]);
    store.apply(Event::MemberListUpdate(Box::new(
        serde_json::from_value(update.clone()).unwrap(),
    )));
    let list = store.member_list(200).unwrap();
    assert_eq!(list.id, "different");
    assert_eq!(list.rows[0], None);
    assert_eq!(
        list.rows[2],
        Some(MemberRow::Group {
            name: "Online".into(),
            count: 0
        })
    );
    assert_eq!(list.rows.len(), 4);
    update["groups"] = json!([{"id":"offline","count":1}]);
    update["ops"] =
        json!([{"op":"SYNC","range":[0,0],"items":[{"group":{"id":"unknown-role","count":1}}]}]);
    store.apply(Event::MemberListUpdate(Box::new(
        serde_json::from_value(update).unwrap(),
    )));
    assert_eq!(store.member_list(200).unwrap().rows.len(), 2);
    assert_eq!(
        store.member_list(200).unwrap().rows[0],
        Some(MemberRow::Group {
            name: "Role".into(),
            count: 1
        })
    );
}

#[test]
fn presence_activity_mapping_and_partial_users_preserve_names() {
    let mut store = Store::from_ready(ready());
    for (value, expected) in [
        ("online", Status::Online),
        ("idle", Status::Idle),
        ("dnd", Status::DoNotDisturb),
        ("offline", Status::Offline),
        ("invisible", Status::Offline),
    ] {
        let presence =
            serde_json::from_value(json!({"user":{"id":"2"},"status":value,"activities":[]}))
                .unwrap();
        assert_eq!(
            store.apply(Event::PresenceUpdate(Box::new(presence))),
            Change::Members { guild_id: None }
        );
        assert!(store.user(2).unwrap().status == expected);
        assert_eq!(store.user(2).unwrap().name, "A Writer");
    }
    for (kind, expected) in [
        (0, "Example Game"),
        (1, "Streaming Example Game"),
        (2, "Listening to Example Game"),
        (3, "Watching Example Game"),
        (5, "Competing in Example Game"),
    ] {
        let presence = serde_json::from_value(
            json!({"user_id":"2","guild_id":"200","activities":[{"type":kind,"name":"Example Game"}]}),
        )
        .unwrap();
        store.apply(Event::PresenceUpdate(Box::new(presence)));
        assert_eq!(store.user(2).unwrap().activity.as_deref(), Some(expected));
    }
    let presence = serde_json::from_value(json!({"user_id":"2","activities":[{"type":0,"name":"Game"},{"type":4,"state":"custom","emoji":{"id":"77","name":"custom_emoji"}}]})).unwrap();
    store.apply(Event::PresenceUpdate(Box::new(presence)));
    assert_eq!(store.user(2).unwrap().activity.as_deref(), Some("custom"));
    let mut message = messages().remove(0);
    message.author.id = 2;
    message.author.username = "writer".into();
    store.set_messages(2002, vec![message]);
    assert_eq!(store.user(2).unwrap().activity.as_deref(), Some("custom"));
}

#[test]
fn primary_guild_tag_maps_and_survives_users_without_the_field() {
    let mut store = Store::from_ready(ready());
    let mut message = messages().remove(0);
    message.author = serde_json::from_value(json!({"id":"2","username":"writer","primary_guild":{"identity_enabled":true,"identity_guild_id":"9","tag":"BYND","badge":"flame"}})).unwrap();
    store.set_messages(2002, vec![message.clone()]);
    let tag = store.user(2).unwrap().tag.clone().unwrap();
    assert_eq!((tag.text.as_str(), tag.guild_id), ("BYND", 9));
    assert!(tag.badge_spec().is_some());
    message.author = serde_json::from_value(json!({"id":"2","username":"writer"})).unwrap();
    store.set_messages(2002, vec![message.clone()]);
    assert!(store.user(2).unwrap().tag.is_some());
    message.author = serde_json::from_value(json!({"id":"2","username":"writer","primary_guild":{"identity_enabled":false,"tag":"BYND"}})).unwrap();
    store.set_messages(2002, vec![message]);
    assert!(store.user(2).unwrap().tag.is_none());
}

#[test]
fn decorations_survive_partial_authors_and_guild_nameplates_override() {
    let mut store = Store::from_ready(ready());
    let mut message = messages().remove(0);
    message.author = serde_json::from_value(json!({
        "id": "2",
        "username": "writer",
        "avatar_decoration_data": {"asset": "a_cat", "sku_id": "7"},
        "collectibles": {"nameplate": {"asset": "nameplates/mouse/1/", "palette": "berry"}},
        "display_name_styles": {"font_id": 14, "effect_id": 8, "colors": [1, 2]}
    }))
    .unwrap();
    store.set_messages(2002, vec![message.clone()]);
    let user = store.user(2).unwrap();
    assert_eq!(user.decoration.as_ref().unwrap().asset, "a_cat");
    assert!(matches!(
        user.decoration.as_ref().unwrap().image_spec(true),
        ImageSpec::AvatarDecoration { animated: true, .. }
    ));
    assert_eq!(user.name_style.as_ref().unwrap().colors, [1, 2]);
    assert_eq!(
        store.nameplate(None, 2).unwrap().asset,
        "nameplates/mouse/1/"
    );

    message.author = serde_json::from_value(json!({"id": "2", "username": "writer"})).unwrap();
    store.set_messages(2002, vec![message.clone()]);
    message.author = serde_json::from_value(json!({"id": "2", "global_name": "W"})).unwrap();
    store.set_messages(2002, vec![message.clone()]);
    let user = store.user(2).unwrap();
    assert!(user.decoration.is_some() && user.nameplate.is_some() && user.name_style.is_some());

    member_event(
        &mut store,
        json!({
            "guild_id": "200",
            "user": {"id": "2"},
            "collectibles": {"nameplate": {"asset": "nameplates/rain/2/"}}
        }),
    );
    assert_eq!(
        store.nameplate(Some(200), 2).unwrap().asset,
        "nameplates/rain/2/"
    );
    assert_eq!(
        store.nameplate(Some(201), 2).unwrap().asset,
        "nameplates/mouse/1/"
    );
    assert_eq!(
        store.nameplate(None, 2).unwrap().asset,
        "nameplates/mouse/1/"
    );
    member_event(
        &mut store,
        json!({"guild_id": "200", "user": {"id": "2"}, "nick": "N"}),
    );
    assert_eq!(
        store.nameplate(Some(200), 2).unwrap().asset,
        "nameplates/rain/2/"
    );
    member_event(
        &mut store,
        json!({"guild_id": "200", "user": {"id": "2"}, "collectibles": null}),
    );
    assert_eq!(
        store.nameplate(Some(200), 2).unwrap().asset,
        "nameplates/mouse/1/"
    );

    message.author = serde_json::from_value(json!({
        "id": "2",
        "global_name": "W",
        "avatar_decoration_data": null,
        "collectibles": null
    }))
    .unwrap();
    store.set_messages(2002, vec![message.clone()]);
    message.author = serde_json::from_value(json!({
        "id": "2",
        "username": "writer",
        "display_name_styles": null
    }))
    .unwrap();
    store.set_messages(2002, vec![message]);
    let user = store.user(2).unwrap();
    assert!(user.decoration.is_none() && user.nameplate.is_none() && user.name_style.is_none());
    assert!(store.nameplate(Some(200), 2).is_none());
}

#[test]
fn offline_profile_applies_server_cosmetics_without_replacing_global_ones() {
    let mut store = Store::from_ready(ready());
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../discord/fixtures/profile_modal.json")).unwrap();
    value["guild_member"]["nick"] = json!("Server nickname");
    value["guild_member"]["avatar"] = json!("server-avatar");
    value["guild_member"]["avatar_decoration_data"] = json!({"asset": "a_server", "sku_id": "7"});
    value["guild_member"]["display_name_styles"] =
        json!({"font_id": 1, "colors": [0x112233, 0x445566]});
    let profile: model::Profile = serde_json::from_value(value.clone()).unwrap();
    let id = profile.user.user.id;
    store.absorb_profile(Some(200), &profile);
    assert!(store.user(id).unwrap().status == Status::Offline);
    assert_eq!(store.member_name(200, id), Some("Server nickname"));
    assert!(matches!(store.member_avatar_spec(200, id),
        Some(ImageSpec::GuildAvatar { hash, .. }) if hash == "server-avatar"));
    assert_eq!(store.member_decoration(200, id).unwrap().asset, "a_server");
    assert_eq!(
        store.member_name_style(200, id).unwrap().colors,
        [0x112233, 0x445566]
    );
    let global = store
        .user(id)
        .unwrap()
        .decoration
        .as_ref()
        .unwrap()
        .asset
        .clone();
    assert_ne!(global, "a_server");
    assert_eq!(store.member_decoration(201, id).unwrap().asset, global);
    assert_ne!(
        store.member_name_style(201, id).unwrap().colors,
        [0x112233, 0x445566]
    );

    // An unrelated partial update must not erase server-specific cosmetics.
    member_event(
        &mut store,
        json!({"guild_id": "200", "user": {"id": id.to_string()}, "nick": "Changed"}),
    );
    assert_eq!(store.member_decoration(200, id).unwrap().asset, "a_server");
    assert_eq!(
        store.member_name_style(200, id).unwrap().colors,
        [0x112233, 0x445566]
    );

    // Explicitly removing an override restores the global cosmetics.
    value["guild_member"]["avatar"] = serde_json::Value::Null;
    value["guild_member"]["avatar_decoration_data"] = serde_json::Value::Null;
    value["guild_member"]["display_name_styles"] = serde_json::Value::Null;
    store.absorb_profile(Some(200), &serde_json::from_value(value).unwrap());
    assert!(store.member_avatar_spec(200, id).is_none());
    assert_eq!(store.member_decoration(200, id).unwrap().asset, global);
    assert_eq!(
        store.member_name_style(200, id).unwrap().colors,
        store.user(id).unwrap().name_style.as_ref().unwrap().colors
    );
}

#[test]
fn profile_retains_nested_user_cosmetics_when_the_top_level_user_omits_them() {
    let mut store = Store::from_ready(ready());
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../discord/fixtures/profile_modal.json")).unwrap();
    for key in [
        "avatar_decoration_data",
        "collectibles",
        "display_name_styles",
    ] {
        value["user"].as_object_mut().unwrap().remove(key);
    }
    let profile: model::Profile = serde_json::from_value(value.clone()).unwrap();
    let id = profile.user.user.id;
    store.absorb_profile(Some(200), &profile);
    let user = store.user(id).unwrap();
    assert!(user.status == Status::Offline);
    assert!(user.decoration.is_some() && user.nameplate.is_some() && user.name_style.is_some());
    assert!(store.nameplate(Some(200), id).is_some());

    value["user"]["avatar_decoration_data"] = serde_json::Value::Null;
    store.absorb_profile(Some(200), &serde_json::from_value(value).unwrap());
    assert!(store.user(id).unwrap().decoration.is_none());
    assert!(store.member_decoration(200, id).is_none());
}

#[test]
fn settings_own_status_partial_updates_do_not_reorder_folders() {
    let mut store = Store::from_ready(ready());
    let rail = format!("{:?}", store.rail);
    for (value, expected) in [
        ("idle", Status::Idle),
        ("invisible", Status::Offline),
        ("dnd", Status::DoNotDisturb),
        ("unknown", Status::Online),
    ] {
        let proto =
            settings_fixtures::status_fixture(Some(value), Some(("working", 0, "\u{1f3ae}", 0)));
        assert_eq!(
            store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
                settings: model::SettingsProto { kind: 1, proto },
                partial: true
            }))),
            Change::Members { guild_id: None }
        );
        assert!(store.user(1).unwrap().status == expected);
        assert_eq!(
            store.user(1).unwrap().activity.as_deref(),
            Some("\u{1f3ae} working")
        );
        assert_eq!(format!("{:?}", store.rail), rail);
    }
    let proto = settings_fixtures::status_fixture(Some("idle"), None);
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto { kind: 1, proto },
        partial: true,
    })));
    assert_eq!(
        store.user(1).unwrap().activity.as_deref(),
        Some("\u{1f3ae} working")
    );
    let proto = settings_fixtures::status_fixture(None, Some(("expired", 0, "", 1)));
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto { kind: 1, proto },
        partial: true,
    })));
    assert!(store.user(1).unwrap().activity.is_none());
    assert!(store.user(1).unwrap().status == Status::Idle);
}

#[test]
fn settings_custom_status_maps_custom_and_unicode_emoji() {
    let mut store = Store::from_ready(ready());
    let proto = settings_fixtures::status_fixture(Some("online"), Some(("", 42, "party", 0)));
    assert_eq!(
        store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
            settings: model::SettingsProto { kind: 1, proto },
            partial: true,
        }))),
        Change::Members { guild_id: None }
    );
    let status = store.user(1).unwrap().custom_status.as_ref().unwrap();
    assert!(status.text.is_none());
    assert!(matches!(
        &status.emoji,
        Some(StatusEmoji::Custom {
            id: 42,
            name,
            animated: false
        }) if name == "party"
    ));
    assert!(status.expires_at.is_none());

    let proto =
        settings_fixtures::status_fixture(Some("online"), Some(("gaming", 0, "\u{1f3ae}", 0)));
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto { kind: 1, proto },
        partial: true,
    })));
    let status = store.user(1).unwrap().custom_status.as_ref().unwrap();
    assert_eq!(status.text.as_deref(), Some("gaming"));
    assert!(matches!(
        &status.emoji,
        Some(StatusEmoji::Unicode(emoji)) if emoji == "\u{1f3ae}"
    ));

    use std::time::{Duration, UNIX_EPOCH};
    let status = CustomStatus {
        text: Some("later".into()),
        emoji: None,
        expires_at: Some(UNIX_EPOCH + Duration::from_secs(1)),
    };
    assert!(!status.is_expired(UNIX_EPOCH));
    assert!(status.is_expired(UNIX_EPOCH + Duration::from_secs(2)));
}

#[test]
fn presence_custom_status_keeps_custom_emoji_without_text() {
    let mut store = Store::from_ready(ready());
    let presence: model::Presence = serde_json::from_value(json!({
        "user_id":"2",
        "activities":[{"type":4,"emoji":{"id":"42","name":"party","animated":true}}]
    }))
    .unwrap();
    assert_eq!(
        store.apply(Event::PresenceUpdate(Box::new(presence))),
        Change::Members { guild_id: None }
    );
    let status = store.user(2).unwrap().custom_status.as_ref().unwrap();
    assert!(status.text.is_none());
    assert!(matches!(
        &status.emoji,
        Some(StatusEmoji::Custom {
            id: 42,
            name,
            animated: true
        }) if name == "party"
    ));
}

#[test]
fn user_update_merges_partial_payload_and_returns_members_change() {
    let mut store = Store::from_ready(ready());
    let avatar = store.user(1).unwrap().avatar.clone();
    let update: model::UserUpdate = serde_json::from_value(json!({
        "id":"1",
        "global_name":"Renamed",
        "avatar_decoration_data":{"asset":"a_new"}
    }))
    .unwrap();
    assert_eq!(
        store.apply(Event::UserUpdate(Box::new(update))),
        Change::Members { guild_id: None }
    );
    let user = store.user(1).unwrap();
    assert_eq!(user.username, "owner");
    assert_eq!(user.name, "Renamed");
    assert_eq!(
        user.decoration
            .as_ref()
            .map(|decoration| decoration.asset.as_str()),
        Some("a_new")
    );
    assert_eq!(user.avatar, avatar);

    let update: model::UserUpdate = serde_json::from_value(json!({
        "id":"1",
        "username":"new_owner",
        "global_name":null,
        "avatar_decoration_data":null
    }))
    .unwrap();
    store.apply(Event::UserUpdate(Box::new(update)));
    let user = store.user(1).unwrap();
    assert_eq!(user.username, "new_owner");
    assert_eq!(user.name, "new_owner");
    assert!(user.decoration.is_none());
}

#[test]
fn reply_previews_hide_only_standalone_media_urls_and_keep_the_original_time() {
    let mut store = Store::from_ready(ready());
    let url = "https://klipy.com/gifs/pardofelis-honkai-impact";
    let timestamp = "2026-10-22T12:57:00Z";
    for (kind, content, embed_url, flags, expected) in [
        ("gifv", url.to_owned(), url, 0, String::new()),
        ("image", format!("  {url}\n"), url, 0, String::new()),
        ("article", url.to_owned(), url, 0, url.to_owned()),
        (
            "gifv",
            format!("Look at this {url}"),
            url,
            0,
            format!("Look at this {url}"),
        ),
        ("gifv", url.to_owned(), url, 4, url.to_owned()),
        (
            "gifv",
            url.to_owned(),
            "https://example.com/different",
            0,
            url.to_owned(),
        ),
    ] {
        let value = json!({
            "id":"500", "channel_id":"2002", "guild_id":"200",
            "author":{"id":"2","username":"writer"},
            "timestamp":"2026-10-22T13:00:00Z", "content":"reply",
            "message_reference":{"message_id":"400","channel_id":"2002"},
            "referenced_message":{
                "id":"400", "author":{"id":"8","username":"original"},
                "timestamp":timestamp, "content":content, "flags":flags,
                "embeds":[{"type":kind,"url":embed_url}]
            }
        });
        store.set_messages(2002, vec![serde_json::from_value(value).unwrap()]);
        let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
        let Some(ReplyState::Loaded(reply)) = &channel.messages[0].reply else {
            panic!("loaded reply expected");
        };
        assert_eq!(reply.content, expected, "kind={kind} flags={flags}");
        assert_eq!(Some(reply.timestamp), local_timestamp(timestamp));
    }
}

#[test]
fn queued_and_forwarded_gif_replies_use_the_same_media_preview_rule() {
    let mut store = Store::from_ready(ready());
    let url = "https://klipy.com/gifs/example";
    let original = json!({
        "id":"400", "channel_id":"2002", "guild_id":"200",
        "author":{"id":"8","username":"original"},
        "timestamp":"2026-10-22T12:57:00Z", "content":url,
        "embeds":[{"type":"gifv","url":url}]
    });
    store.set_messages(2002, vec![serde_json::from_value(original).unwrap()]);
    store.queue_message(2002, 600, "reply".into(), Some(400));
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    assert_eq!(channel.messages[0].content, url);
    let Some(ReplyState::Loaded(reply)) = &channel.messages[1].reply else {
        panic!("queued reply expected");
    };
    assert!(reply.content.is_empty() && reply.embeds);
    assert_eq!(reply.timestamp, channel.messages[0].timestamp);

    let forwarded = json!({
        "id":"700", "channel_id":"2002", "guild_id":"200",
        "author":{"id":"2","username":"writer"},
        "timestamp":"2026-10-22T13:00:00Z", "content":"reply",
        "message_reference":{"message_id":"500","channel_id":"2002"},
        "referenced_message":{
            "id":"500", "author":{"id":"8","username":"original"},
            "content":"", "message_snapshots":[{"message":{
                "content":url, "embeds":[{"type":"gifv","url":url}]
            }}]
        }
    });
    store.set_messages(2002, vec![serde_json::from_value(forwarded).unwrap()]);
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    let Some(ReplyState::Loaded(reply)) = &channel.messages[0].reply else {
        panic!("forwarded reply expected");
    };
    assert!(reply.content.is_empty() && reply.embeds);
}

#[test]
fn replies_mentions_and_partial_message_updates() {
    let mut store = Store::from_ready(ready());
    let base = json!({"id":"500","channel_id":"2002","guild_id":"200","author":{"id":"2","username":"writer"},"timestamp":"2026-10-22T00:00:00Z","content":"reply","member":{"nick":"Message Nick","roles":["201"]},"message_reference":{"message_id":"400","channel_id":"2002","guild_id":"200"}});
    for (referenced, expected) in [
        (None, ReplyState::Unavailable),
        (Some(serde_json::Value::Null), ReplyState::Deleted),
        (
            Some(
                json!({"id":"400","author":{"id":"8","username":"reply-author"},"content":"original","attachments":[{"id":"1"}],"embeds":[{}],"referenced_message":{"author":"malformed nested object is ignored"}}),
            ),
            ReplyState::Loaded(Reply {
                message_id: 400,
                author_id: 8,
                timestamp: timestamp_from_nonce(400),
                content: "original".into(),
                attachments: true,
                embeds: true,
                sticker: None,
            }),
        ),
    ] {
        let mut value = base.clone();
        if let Some(referenced) = referenced {
            value["referenced_message"] = referenced;
        }
        let message = serde_json::from_value(value).unwrap();
        store.set_messages(2002, vec![message]);
        assert_eq!(
            store
                .guild_by_id(200)
                .unwrap()
                .channel(2002)
                .unwrap()
                .messages[0]
                .reply,
            Some(expected)
        );
    }
    assert!(store.user(8).is_some());
    assert_eq!(store.member_name(200, 2), Some("Message Nick"));
    let mut forwarded = base.clone();
    forwarded["message_reference"]["type"] = 1.into();
    forwarded["message_snapshots"] = json!([{
        "message": {
            "content": "forwarded text",
            "embeds": [],
            "attachments": [],
            "components": [],
            "flags": 0,
            "type": 0,
            "timestamp": "2026-10-22T02:00:00Z",
            "edited_timestamp": null
        }
    }]);
    store.set_messages(2002, vec![serde_json::from_value(forwarded).unwrap()]);
    let message = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages[0];
    assert!(message.reply.is_none());
    let forward = message.forward.as_ref().unwrap();
    assert_eq!(forward.content, "forwarded text");
    assert_eq!(forward.channel_id, Some(2002));
    assert_eq!(forward.guild_id, Some(200));
    assert!(forward.timestamp.is_some() && !forward.edited);
    for fields in [
        json!({"mentions":[{"id":"1"}]}),
        json!({"mention_roles":["201"]}),
        json!({"mention_everyone":true}),
    ] {
        let mut value = base.clone();
        value
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let message: model::Message = serde_json::from_value(value).unwrap();
        store.set_messages(2002, vec![]);
        store.apply(Event::MessageCreate(Box::new(message)));
        assert!(
            store
                .guild_by_id(200)
                .unwrap()
                .channel(2002)
                .unwrap()
                .messages[0]
                .mentions_me
        );
        let update = serde_json::from_value(json!({"id":"500","channel_id":"2002","edited_timestamp":"2026-10-22T01:00:00Z","type":7})).unwrap();
        store.apply(Event::MessageUpdate(Box::new(update)));
        let message = &store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages[0];
        assert!(message.mentions_me && message.edited);
        assert_eq!(message.kind, 7);
        let update = serde_json::from_value(json!({"id":"500","channel_id":"2002","mentions":[],"mention_roles":[],"mention_everyone":false,"referenced_message":null,"member":{"nick":"Updated Nick"}})).unwrap();
        store.apply(Event::MessageUpdate(Box::new(update)));
        let message = &store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages[0];
        assert!(!message.mentions_me);
        assert_eq!(message.reply, Some(ReplyState::Deleted));
        assert_eq!(store.member_name(200, 2), Some("Updated Nick"));
    }
}

#[test]
fn sticker_items_map_into_messages_forwards_and_updates() {
    let mut store = Store::from_ready(ready());
    let message = json!({
        "id": "500",
        "channel_id": "2002",
        "guild_id": "200",
        "author": {"id": "2", "username": "writer"},
        "timestamp": "2026-10-22T00:00:00Z",
        "content": "",
        "sticker_items": [
            {"id": "11", "name": "wave", "format_type": 1},
            {"id": "12", "name": "dance", "format_type": 2}
        ],
        "message_reference": {"type": 1, "channel_id": "2002", "guild_id": "200"},
        "message_snapshots": [{"message": {
            "content": "",
            "sticker_items": [{"id": "13", "name": "forwarded", "format_type": 4}]
        }}]
    });
    store.set_messages(2002, vec![serde_json::from_value(message).unwrap()]);
    let message = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages[0];
    let stickers: Vec<_> = message
        .stickers
        .iter()
        .map(|sticker| (sticker.id, sticker.name.as_str(), sticker.format))
        .collect();
    assert_eq!(stickers, [(11, "wave", 1), (12, "dance", 2)]);
    let forward = message.forward.as_ref().unwrap();
    assert_eq!(forward.stickers[0].id, 13);
    assert_eq!(forward.stickers[0].name, "forwarded");
    assert_eq!(forward.stickers[0].format, 4);

    let update: model::MessageUpdate = serde_json::from_value(json!({
        "id": "500",
        "channel_id": "2002",
        "sticker_items": [{"id": "14", "name": "edited", "format_type": 3}]
    }))
    .unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    let message = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages[0];
    assert_eq!(message.stickers.len(), 1);
    assert_eq!(
        (
            message.stickers[0].id,
            message.stickers[0].name.as_str(),
            message.stickers[0].format
        ),
        (14, "edited", 3)
    );
}

#[test]
fn sticker_only_reply_preview_uses_sticker_name() {
    let mut store = Store::from_ready(ready());
    let message = json!({
        "id": "500",
        "channel_id": "2002",
        "guild_id": "200",
        "author": {"id": "2", "username": "writer"},
        "timestamp": "2026-10-22T00:00:00Z",
        "message_reference": {"message_id": "400", "channel_id": "2002", "guild_id": "200"},
        "referenced_message": {
            "id": "400",
            "author": {"id": "8", "username": "reply-author"},
            "content": "",
            "sticker_items": [{"id": "9", "name": "wave", "format_type": 1}]
        }
    });
    store.set_messages(2002, vec![serde_json::from_value(message).unwrap()]);
    let Some(ReplyState::Loaded(reply)) = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages[0]
        .reply
    else {
        panic!("sticker reply expected");
    };
    assert_eq!(reply.sticker.as_deref(), Some("wave"));
    assert!(!reply.attachments && !reply.embeds);
}

#[test]
fn message_update_replaces_forward_snapshot() {
    let mut store = Store::from_ready(ready());
    let message = json!({"id":"500","channel_id":"2002","guild_id":"200","author":{"id":"2","username":"writer"},"timestamp":"2026-10-22T00:00:00Z","message_reference":{"type":1,"channel_id":"2002","guild_id":"200"},"message_snapshots":[{"message":{"content":"first"}}]});
    store.set_messages(2002, vec![serde_json::from_value(message).unwrap()]);
    let update: model::MessageUpdate = serde_json::from_value(json!({
        "id": "500",
        "channel_id": "2002",
        "message_snapshots": [{"message": {"content": "second"}}]
    }))
    .unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    let forward = store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages[0]
        .forward
        .as_ref()
        .unwrap();
    assert_eq!(forward.content, "second");
    assert_eq!(forward.channel_id, Some(2002));
}

#[test]
fn reply_to_forward_previews_snapshot() {
    let mut store = Store::from_ready(ready());
    let message = json!({
        "id": "500",
        "channel_id": "2002",
        "guild_id": "200",
        "author": {"id": "2", "username": "writer"},
        "timestamp": "2026-10-22T00:00:00Z",
        "message_reference": {"message_id": "400", "channel_id": "2002", "guild_id": "200"},
        "referenced_message": {
            "id": "400",
            "author": {"id": "8", "username": "reply-author"},
            "content": "",
            "attachments": [],
            "embeds": [],
            "message_reference": {"type": 1, "channel_id": "2002", "guild_id": "200"},
            "message_snapshots": [{"message": {
                "content": "forwarded preview",
                "attachments": [{"id": "1"}],
                "embeds": [{}],
                "sticker_items": [{"id": "9", "name": "forwarded sticker", "format_type": 2}]
            }}]
        }
    });
    store.set_messages(2002, vec![serde_json::from_value(message).unwrap()]);
    assert_eq!(
        store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages[0]
            .reply,
        Some(ReplyState::Loaded(Reply {
            message_id: 400,
            author_id: 8,
            timestamp: timestamp_from_nonce(400),
            content: "forwarded preview".into(),
            attachments: true,
            embeds: true,
            sticker: Some("forwarded sticker".into()),
        }))
    );
}

#[test]
fn all_channel_names_and_seed_social_data() {
    let mut store = Store::from_ready(ready());
    assert_eq!(store.channel_name(210), Some("Later"));
    assert_eq!(store.channel_name(2005), Some("stage"));
    assert_eq!(store.channel_name(2001), Some("hidden"));
    assert_eq!(store.channel_name(900), Some("A Writer"));
    let thread =
        serde_json::from_value(json!({"id":"777","guild_id":"200","type":11,"name":"thread"}))
            .unwrap();
    store.apply(Event::ChannelUpsert(Box::new(thread)));
    assert_eq!(store.channel_name(777), Some("thread"));
    store.apply(Event::ChannelDelete { id: 777 });
    assert!(store.channel_name(777).is_none());
    let store = Store::seed();
    assert_eq!(store.member_name(1, 2), Some("Demo Two"));
    assert!(store.member_color(1, 1).is_some());
    assert!(
        store
            .guilds
            .iter()
            .all(|g| store.member_list(g.id).is_some())
    );
    let messages: Vec<_> = store
        .guilds
        .iter()
        .flat_map(|g| g.channels())
        .flat_map(|c| &c.messages)
        .collect();
    assert!(messages.iter().any(|m| m.kind == 7));
    assert!(messages.iter().any(|m| m.edited));
    assert!(
        messages
            .iter()
            .any(|m| matches!(m.reply, Some(ReplyState::Loaded(_))))
    );
}

#[test]
fn prepend_messages_inserts_only_older_messages_in_order() {
    let mut store = Store::from_ready(ready());
    let mut page = messages();
    page.retain(|message| message.channel_id == 2002);
    page.sort_by_key(|message| message.id);
    let (older, newer) = page.split_at(page.len() / 2);
    assert!(!older.is_empty() && !newer.is_empty());
    store.set_messages(2002, newer.to_vec());
    let mut fetched = page.clone();
    fetched.reverse();
    assert_eq!(store.prepend_messages(2002, fetched), older.len());
    let ids: Vec<_> = store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages
        .iter()
        .map(|message| message.id)
        .collect();
    assert_eq!(
        ids,
        page.iter().map(|message| message.id).collect::<Vec<_>>()
    );
    assert_eq!(store.prepend_messages(2002, older.to_vec()), 0);
}

#[test]
fn open_at_detached_channel_ignores_live_message() {
    let mut store = Store::from_ready(ready());
    store.set_around_messages(2002, vec![server_message(55, 2002, 2, None)]);
    assert!(!store.at_present(2002));
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(server_message(
            70, 2002, 3, None
        )))),
        Change::Structure
    );
    let channel = store.guild_by_id(200).unwrap().channel(2002).unwrap();
    assert_eq!(
        channel
            .messages
            .iter()
            .map(|message| message.id)
            .collect::<Vec<_>>(),
        [55]
    );
    assert!(channel.unread);
    assert_eq!(store.channel_ack(2002), Some(70));
    assert_eq!(
        store.queue_message(2002, 90, "mine".into(), None),
        Change::None
    );

    store.set_messages(2002, vec![server_message(55, 2002, 2, None)]);
    assert!(store.at_present(2002));
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(server_message(
            70, 2002, 3, None
        )))),
        Change::MessageInserted {
            channel_id: 2002,
            index: 1
        }
    );
    let ids: Vec<_> = store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages
        .iter()
        .map(|message| message.id)
        .collect();
    assert_eq!(ids, [55, 70]);
    assert_eq!(
        store.queue_message(2002, 90, "mine".into(), None),
        Change::MessageInserted {
            channel_id: 2002,
            index: 2
        }
    );
}

#[test]
fn open_at_append_messages_dedups_and_short_page_marks_present() {
    let mut store = Store::from_ready(ready());
    store.set_around_messages(
        2002,
        vec![
            server_message(55, 2002, 2, None),
            server_message(56, 2002, 2, None),
        ],
    );
    assert!(!store.at_present(2002));
    assert_eq!(
        store.append_messages(
            2002,
            vec![
                server_message(57, 2002, 2, None),
                server_message(56, 2002, 2, None),
                server_message(58, 2002, 2, None),
            ],
            50,
        ),
        2
    );
    let ids: Vec<_> = store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages
        .iter()
        .map(|message| message.id)
        .collect();
    assert_eq!(ids, [55, 56, 57, 58]);
    assert!(store.at_present(2002));

    let mut store = Store::from_ready(ready());
    store.set_around_messages(2002, vec![server_message(55, 2002, 2, None)]);
    assert_eq!(
        store.append_messages(
            2002,
            vec![
                server_message(57, 2002, 2, None),
                server_message(58, 2002, 2, None),
            ],
            2,
        ),
        2
    );
    assert!(!store.at_present(2002));
    assert_eq!(store.append_messages(2002, Vec::new(), 50), 0);
    assert!(store.at_present(2002));
}

#[test]
fn open_at_settings_without_new_fields_load() {
    let settings: crate::settings::Settings = serde_json::from_str(
        r#"{"zoom":4,"window":{"x":1.0,"y":2.0,"width":800.0,"height":600.0,"maximized":false}}"#,
    )
    .unwrap();
    assert_eq!(settings.zoom, Some(4));
    assert!(settings.last_channel.is_none());
    assert!(settings.guild_channels.is_empty());

    let saved: crate::settings::Settings = serde_json::from_value(json!({
        "last_channel": {"guild_id": 200, "channel_id": 2002},
        "guild_channels": {"200": 2002}
    }))
    .unwrap();
    assert_eq!(saved.last_channel.unwrap().channel_id, 2002);
    assert_eq!(saved.guild_channels.get(&200), Some(&2002));
    let round_trip: crate::settings::Settings =
        serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
    assert!(round_trip == saved);
}

fn custom_emoji(id: u64, guild_id: u64, name: &str) -> CustomEmoji {
    CustomEmoji {
        id,
        name: name.to_owned(),
        animated: false,
        guild_id,
        available: true,
        roles: Vec::new(),
    }
}

fn guild_emoji(id: u64, name: &str) -> model::GuildEmoji {
    model::GuildEmoji {
        id,
        name: name.to_owned(),
        animated: false,
        available: true,
        roles: Vec::new(),
        require_colons: true,
    }
}

#[test]
fn emoji_api_access_rules() {
    let store = Store::from_ready(ready());
    let usable = custom_emoji(1, 200, "usable");
    assert_eq!(store.emoji_access(&usable, 200, 2002), EmojiAccess::Usable);

    let mut unavailable = usable.clone();
    unavailable.available = false;
    assert_eq!(
        store.emoji_access(&unavailable, 200, 2002),
        EmojiAccess::Unavailable
    );

    let mut restricted = usable.clone();
    restricted.roles = vec![999];
    assert_eq!(
        store.emoji_access(&restricted, 200, 2002),
        EmojiAccess::NeedsRole
    );

    let mut animated = usable.clone();
    animated.animated = true;
    assert_eq!(
        store.emoji_access(&animated, 200, 2002),
        EmojiAccess::NeedsNitro
    );

    let external = custom_emoji(2, 100, "external");
    assert_eq!(
        store.emoji_access(&external, 200, 2002),
        EmojiAccess::NeedsNitro
    );

    let mut ready = ready();
    ready.user.premium_type = Some(2);
    let store = Store::from_ready(ready);
    assert!(store.has_nitro());
    assert_eq!(
        store.emoji_access(&animated, 200, 2002),
        EmojiAccess::Usable
    );
    assert_eq!(
        store.emoji_access(&external, 200, 2002),
        EmojiAccess::NoExternalPermission
    );
}

#[test]
fn emoji_api_search_orders_current_guild_then_others_then_unicode() {
    let mut ready = ready();
    ready
        .guilds
        .iter_mut()
        .find(|guild| guild.id == 200)
        .unwrap()
        .emojis = Some(vec![
        guild_emoji(1, "qqq_prefix"),
        guild_emoji(2, "xqqq_sub"),
    ]);
    ready
        .guilds
        .iter_mut()
        .find(|guild| guild.id == 100)
        .unwrap()
        .emojis = Some(vec![guild_emoji(3, "qqq_other")]);
    let store = Store::from_ready(ready);

    let results = store.search_emoji("qqq", 200, 2002, 10);
    let names: Vec<_> = results
        .iter()
        .map(|(choice, _)| match choice {
            EmojiChoice::Custom(emoji) => emoji.name.as_str(),
            EmojiChoice::Unicode { shortcode, .. } => shortcode.as_str(),
        })
        .collect();
    assert_eq!(names, ["qqq_prefix", "xqqq_sub", "qqq_other"]);
    assert_eq!(results[2].1, EmojiAccess::NeedsNitro);
    assert!(store.search_emoji("", 200, 2002, 10).is_empty());

    let unicode = store.search_emoji("wave", 200, 2002, 3);
    assert!(matches!(
        &unicode[0].0,
        EmojiChoice::Unicode { shortcode, .. } if shortcode == "wave"
    ));
    assert_eq!(unicode[0].1, EmojiAccess::Usable);
}

#[test]
fn emoji_api_guild_emojis_update_replaces_and_skips_malformed() {
    let mut store = Store::from_ready(ready());
    let update: model::GuildEmojisUpdate = serde_json::from_value(json!({
        "guild_id": "200",
        "emojis": [
            {"id":"9001","name":"party","animated":true,"available":false,
             "roles":["7"],"require_colons":true},
            {"name":"missing-id"},
            {"id":"bad","name":"bad-id"},
            {"id":"9002","name":"plain"}
        ]
    }))
    .unwrap();
    assert_eq!(
        store.apply(Event::GuildEmojisUpdate(update)),
        Change::Structure
    );
    let guild = store.guild_by_id(200).unwrap();
    let names: Vec<_> = guild
        .emojis
        .iter()
        .map(|emoji| emoji.name.as_str())
        .collect();
    assert_eq!(names, ["party", "plain"]);
    assert!(guild.emojis[0].animated && !guild.emojis[0].available);
    assert_eq!(guild.emojis[0].roles, [7]);

    let update: model::GuildEmojisUpdate = serde_json::from_value(json!({
        "guild_id": "200",
        "emojis": [{"id":"9003","name":"replacement"}]
    }))
    .unwrap();
    store.apply(Event::GuildEmojisUpdate(update));
    let names: Vec<_> = store
        .guild_by_id(200)
        .unwrap()
        .emojis
        .iter()
        .map(|emoji| emoji.name.as_str())
        .collect();
    assert_eq!(names, ["replacement"]);
}

#[test]
fn emoji_api_reaction_echo_does_not_double_count() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    let emoji = ReactionEmoji {
        id: None,
        name: "\u{1F44D}".into(),
        animated: false,
    };
    assert_ne!(store.set_reaction(2002, 61, &emoji, true), Change::None);
    let event: model::ReactionEvent = serde_json::from_value(json!({
        "channel_id": "2002",
        "message_id": "61",
        "user_id": "1",
        "emoji": {"name": "\u{1F44D}"}
    }))
    .unwrap();
    store.apply(Event::MessageReactionAdd(event));
    let reaction = store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages
        .iter()
        .find(|message| message.id == 61)
        .unwrap()
        .reactions
        .iter()
        .find(|reaction| reaction.emoji.name == "\u{1F44D}")
        .unwrap();
    assert_eq!(reaction.count, 1);
    assert!(reaction.me);
}

fn server_message(id: u64, channel_id: u64, author_id: u64, nonce: Option<u64>) -> model::Message {
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

#[test]
fn queued_message_is_replaced_by_rest_then_gateway_duplicate() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    assert_eq!(
        store.queue_message(2002, 1000, "hello".into(), Some(61)),
        Change::MessageInserted {
            channel_id: 2002,
            index: 2
        }
    );
    let queued = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages[2];
    assert_eq!(queued.delivery, Delivery::Pending);
    assert_eq!(queued.author_id, 1);
    assert!(matches!(&queued.reply, Some(ReplyState::Loaded(reply)) if reply.message_id == 61));
    assert_eq!(
        store.queue_message(2002, 1002, "missing".into(), Some(9999)),
        Change::MessageInserted {
            channel_id: 2002,
            index: 3
        }
    );
    assert!(matches!(
        &store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages[3]
            .reply,
        Some(ReplyState::Unavailable)
    ));
    store.discard_message(2002, 1002);

    let created = server_message(1001, 2002, 1, Some(1000));
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(created.clone()))),
        Change::MessageReplaced {
            channel_id: 2002,
            removed: 2,
            inserted: 2
        }
    );
    let messages = &store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages;
    assert_eq!(messages[2].delivery, Delivery::Sent);
    assert_eq!(messages[2].id, 1001);
    assert_eq!(store.last_read(2002), Some(1001));
    assert!(
        !store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .unread
    );
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(created))),
        Change::None
    );
}

#[test]
fn queued_message_is_replaced_by_gateway_then_rest_duplicate() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    store.queue_message(2002, 2000, "first".into(), None);
    let created = server_message(2001, 2002, 1, Some(2000));
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(created.clone()))),
        Change::MessageReplaced {
            channel_id: 2002,
            removed: 2,
            inserted: 2
        }
    );
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(created))),
        Change::None
    );
}

#[test]
fn replacement_indices_account_for_messages_arriving_between() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    store.queue_message(2002, 1000, "mine".into(), None);
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(server_message(
            1500, 2002, 2, None
        )))),
        Change::MessageInserted {
            channel_id: 2002,
            index: 3
        }
    );
    assert_eq!(
        store.apply(Event::MessageCreate(Box::new(server_message(
            2000,
            2002,
            1,
            Some(1000)
        )))),
        Change::MessageReplaced {
            channel_id: 2002,
            removed: 2,
            inserted: 3
        }
    );
    let ids: Vec<_> = store
        .guild_by_id(200)
        .unwrap()
        .channel(2002)
        .unwrap()
        .messages
        .iter()
        .map(|message| message.id)
        .collect();
    assert_eq!(ids, [61, 62, 1500, 2000]);
}

#[test]
fn failed_and_discarded_messages_and_sent_rows_stay() {
    let mut store = Store::from_ready(ready());
    store.set_messages(2002, messages());
    store.queue_message(2002, 5000, "oops".into(), None);
    assert_eq!(
        store.fail_message(2002, 5000),
        Change::MessageChanged {
            channel_id: 2002,
            index: 2
        }
    );
    assert_eq!(
        store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages[2]
            .delivery,
        Delivery::Failed
    );
    assert_eq!(store.fail_message(2002, 5000), Change::None);
    assert_eq!(store.discard_message(2002, 61), Change::None);
    assert_eq!(
        store.discard_message(2002, 5000),
        Change::MessageRemoved {
            channel_id: 2002,
            index: 2
        }
    );
    assert_eq!(store.discard_message(2002, 5000), Change::None);
    assert_eq!(
        store
            .guild_by_id(200)
            .unwrap()
            .channel(2002)
            .unwrap()
            .messages
            .len(),
        2
    );
}

#[test]
fn channel_permissions_gate_sending_and_managing() {
    let store = Store::from_ready(ready());
    assert!(!store.can_send(2002));
    assert!(!store.can_manage_messages(2002));
    assert!(store.can_send(1001));
    assert!(store.can_manage_messages(1001));
    let demo = Store::seed();
    assert!(demo.can_send(0));
    assert!(demo.can_manage_messages(0));
}

#[test]
fn channel_permission_memo_is_reused_and_invalidated_by_role_updates() {
    let mut store = Store::from_ready(ready());
    assert!(!store.can_send(2002));
    assert_eq!(
        store.permission_cache.borrow().get(&2002),
        Some(&crate::discord::VIEW_CHANNEL)
    );
    assert!(!store.can_send(2002));
    assert_eq!(
        store.permission_cache.borrow().get(&2002),
        Some(&crate::discord::VIEW_CHANNEL)
    );

    assert_eq!(
        role_event(
            &mut store,
            json!({"guild_id":"200","role":{"id":"201","name":"Members","permissions":"2048","position":1}})
        ),
        Change::Structure
    );
    assert!(store.permission_cache.borrow().is_empty());
    assert!(store.can_send(2002));
    assert!(store.can_send(2004));

    assert_eq!(
        member_event(
            &mut store,
            json!({"guild_id":"200","user":{"id":"1"},"roles":[]})
        ),
        Change::Structure
    );
    assert!(store.permission_cache.borrow().is_empty());
    assert!(!store.can_send(2004));
}

#[test]
fn typing_start_registers_member_without_state_change() {
    let mut store = Store::from_ready(ready());
    let typing: model::TypingStart = serde_json::from_value(json!({
        "channel_id": "2002", "guild_id": "200", "user_id": "77",
        "timestamp": 1770000000,
        "member": {"user": {"id": "77", "username": "typer"}, "roles": []}
    }))
    .unwrap();
    assert_eq!(
        store.apply(Event::TypingStart(Box::new(typing))),
        Change::None
    );
    assert_eq!(store.member_name(200, 77), Some("typer"));
}
