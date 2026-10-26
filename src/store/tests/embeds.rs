use super::*;

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
        (Some("article"), EmbedKind::Article),
        (Some("link"), EmbedKind::Link),
        (Some("future"), EmbedKind::Link),
    ] {
        let mut wire = model::Embed {
            kind: kind.map(str::to_owned),
            ..Default::default()
        };
        assert!(mapping::map_embed(&wire).kind == expected);
        wire.color = Some(0);
        // Articles keep their layout with a colour; Discord still puts the thumbnail below.
        if matches!(kind, Some("link" | "future")) {
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
        serde_json::from_str(include_str!("../fixtures/reddit_embeds.json")).unwrap();
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
        "embeds": serde_json::from_str::<serde_json::Value>(include_str!("../fixtures/reddit_embeds.json")).unwrap()["embeds"].clone()
    })).unwrap();
    let forward = mapping::map_snapshot(&snapshot, Some(2002), Some(200));
    assert_eq!(forward.embeds[0].components.len(), 1);
    assert!(mapping::map_embeds(&source.embeds, 4).is_empty());
}

#[test]
fn component_embed_updates_are_not_lost_or_merged_into_other_cards() {
    let mut store = Store::from_ready(ready());
    let source = serde_json::from_str(include_str!("../fixtures/reddit_embeds.json")).unwrap();
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
