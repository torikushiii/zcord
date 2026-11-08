use super::{AckManual, forward_body, message_body, parse_search_reply, parse_search_response};
use crate::{Error, OutgoingMessage, SearchOutcome, SearchScope};
use std::time::Duration;

#[test]
fn message_body_serializes_sticker_ids_only_when_non_empty() {
    let message = OutgoingMessage {
        content: String::new(),
        nonce: 7,
        reply: None,
        attachments: Vec::new(),
        sticker_ids: Vec::new(),
        content_inventory_entry: None,
    };
    let body = serde_json::to_string(&message_body(None, 20, &message)).unwrap();
    assert_eq!(
        body,
        r#"{"mobile_network_type":"unknown","content":"","nonce":"7","tts":false,"flags":0}"#
    );
    let message = OutgoingMessage {
        content: String::new(),
        nonce: 7,
        reply: None,
        attachments: Vec::new(),
        sticker_ids: vec![9001, 9002],
        content_inventory_entry: None,
    };
    let body = serde_json::to_string(&message_body(None, 20, &message)).unwrap();
    assert_eq!(
        body,
        r#"{"mobile_network_type":"unknown","content":"","nonce":"7","tts":false,"flags":0,"sticker_ids":["9001","9002"]}"#
    );
}

#[test]
fn message_body_wraps_the_content_inventory_entry_verbatim() {
    let entry = serde_json::json!({
        "id": "1191169321074688097", "author_id": "1191169170079744061",
        "author_type": 0, "content_type": 1,
        "traits": [{"type": 3, "is_live": true}],
        "extra": {"type": "played_game_extra", "game_name": "Synthetic"},
        "participants": ["1191169170079744061"],
        "signature": "synthetic-signature"
    });
    let message = OutgoingMessage {
        content: "wave".to_owned(),
        nonce: 7,
        reply: None,
        attachments: Vec::new(),
        sticker_ids: Vec::new(),
        content_inventory_entry: Some(std::sync::Arc::new(entry)),
    };
    let body = serde_json::to_string(&message_body(None, 20, &message)).unwrap();
    assert_eq!(
        body,
        r#"{"mobile_network_type":"unknown","content":"wave","nonce":"7","tts":false,"flags":0,"content_inventory_entry":{"unverified_content":{"id":"1191169321074688097","author_id":"1191169170079744061","author_type":0,"content_type":1,"traits":[{"type":3,"is_live":true}],"extra":{"type":"played_game_extra","game_name":"Synthetic"},"participants":["1191169170079744061"],"signature":"synthetic-signature"}}}"#
    );
}

#[test]
fn forward_body_matches_discord() {
    let body = forward_body(Some(200), 2002, 3003, 44);
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"content":"","nonce":"44","tts":false,"flags":0,"mobile_network_type":"unknown","message_reference":{"guild_id":"200","channel_id":"2002","message_id":"3003","type":1}}"#
    );
}

#[test]
fn ack_manual_body_matches_discord() {
    let body = AckManual {
        manual: true,
        mention_count: 3,
    };
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"manual":true,"mention_count":3}"#
    );
}

#[test]
fn search_dms_reply_maps_only_messages_tab_and_preserves_channels() {
    let message = |id: &str, channel_id: &str| {
        serde_json::json!({
            "id": id,
            "channel_id": channel_id,
            "author": {"id": "11"},
            "timestamp": "2024-01-01T00:00:00.000Z"
        })
    };
    let value = serde_json::json!({
        "analytics_id": "search-id",
        "tabs": {
            "messages": {
                "total_results": 42,
                "messages": [
                    [message("1", "10"), message("9", "10")],
                    [],
                    [null, message("8", "20")],
                    [message("2", "20")]
                ],
                "channels": [
                    {"id": "10", "type": 1, "recipients": [{"id": "11"}]},
                    null,
                    {"id": "20", "type": 3, "name": "Group DM"}
                ],
                "cursor": {"type": "timestamp", "timestamp": "2"}
            },
            "media": {"total_results": 1, "messages": [[message("3", "30")]]}
        }
    });
    let outcome = parse_search_reply(200, value, SearchScope::Dms).unwrap();
    let SearchOutcome::Page(page) = outcome else {
        panic!("successful DM search must return a page");
    };
    assert_eq!(page.total, 42);
    assert_eq!(
        page.messages
            .iter()
            .map(|message| (message.id, message.channel_id))
            .collect::<Vec<_>>(),
        [(1, 10), (2, 20)]
    );
    assert_eq!(
        page.channels
            .iter()
            .map(|channel| channel.id)
            .collect::<Vec<_>>(),
        [10, 20]
    );
    assert_eq!(page.channels[0].recipients[0].id, 11);
}

#[test]
fn search_dms_reply_requires_messages_tab_but_accepts_empty_results() {
    for value in [
        serde_json::json!({}),
        serde_json::json!({"messages": [], "total_results": 0}),
        serde_json::json!({"tabs": {}}),
        serde_json::json!({"tabs": {"messages": null}}),
        serde_json::json!({"tabs": {"messages": []}}),
    ] {
        assert!(matches!(
            parse_search_reply(200, value, SearchScope::Dms),
            Err(Error::Decode)
        ));
    }
    let value = serde_json::json!({
        "tabs": {"messages": {
            "total_results": 0, "messages": [], "channels": [], "cursor": {}
        }}
    });
    let outcome = parse_search_reply(200, value, SearchScope::Dms).unwrap();
    let SearchOutcome::Page(page) = outcome else {
        panic!("empty DM search must return a page");
    };
    assert_eq!(page.total, 0);
    assert!(page.messages.is_empty());
    assert!(page.channels.is_empty());
}

#[test]
fn search_dms_indexing_reply_is_parsed_before_the_tabs_envelope() {
    for (status, value) in [
        (202, serde_json::Value::Null),
        (202, serde_json::json!({"retry_after": 2})),
        (200, serde_json::json!({"code": 110000, "retry_after": 2})),
    ] {
        assert!(matches!(
            parse_search_reply(status, value, SearchScope::Dms),
            Ok(SearchOutcome::Indexing { retry_after }) if retry_after == Duration::from_secs(2)
        ));
    }
}

#[test]
fn search_202_reply_returns_clamped_indexing_delay() {
    let outcome = parse_search_reply(
        202,
        serde_json::json!({"code": 110000, "retry_after": 1.5}),
        SearchScope::Channel(10),
    )
    .unwrap();
    assert!(matches!(
        outcome,
        SearchOutcome::Indexing { retry_after } if retry_after == Duration::from_millis(1500)
    ));

    for (requested, expected) in [
        (-1.0, 1),
        (0.2, 1),
        (1.0, 1),
        (30.0, 30),
        (31.0, 30),
        (1e100, 30),
    ] {
        let outcome = parse_search_reply(
            202,
            serde_json::json!({"retry_after": requested}),
            SearchScope::Channel(10),
        )
        .unwrap();
        let SearchOutcome::Indexing { retry_after } = outcome else {
            panic!("202 response must be treated as indexing");
        };
        assert_eq!(retry_after, Duration::from_secs(expected));
    }
}

#[test]
fn search_indexing_code_and_missing_retry_delay_are_tolerated() {
    for code in [110000, 115000, 119999] {
        let outcome = parse_search_reply(
            200,
            serde_json::json!({"code": code}),
            SearchScope::Channel(10),
        )
        .unwrap();
        assert!(matches!(
            outcome,
            SearchOutcome::Indexing { retry_after } if retry_after == Duration::from_secs(2)
        ));
        let outcome = parse_search_reply(
            200,
            serde_json::json!({"code": code, "retry_after": 4.5}),
            SearchScope::Channel(10),
        )
        .unwrap();
        assert!(matches!(
            outcome,
            SearchOutcome::Indexing { retry_after } if retry_after == Duration::from_millis(4500)
        ));
    }
    for value in [
        serde_json::Value::Null,
        serde_json::json!({}),
        serde_json::json!({"code": 50035, "retry_after": null}),
        serde_json::json!({"retry_after": "invalid"}),
    ] {
        let outcome = parse_search_reply(202, value, SearchScope::Channel(10)).unwrap();
        assert!(matches!(
            outcome,
            SearchOutcome::Indexing { retry_after } if retry_after == Duration::from_secs(2)
        ));
    }
    for code in [109999, 120000] {
        assert!(matches!(
            parse_search_reply(
                200,
                serde_json::json!({"code": code}),
                SearchScope::Channel(10),
            ),
            Ok(SearchOutcome::Page(_))
        ));
    }
}

#[tokio::test]
async fn search_rejected_indexing_codes_use_the_default_delay() {
    for code in [110000, 115000, 119999] {
        let outcome = parse_search_response(
            Err(Error::Rejected {
                status: 400,
                code,
                message: "Indexing".to_owned(),
            }),
            SearchScope::Channel(10),
        )
        .await
        .unwrap();
        assert!(matches!(
            outcome,
            SearchOutcome::Indexing { retry_after } if retry_after == Duration::from_secs(2)
        ));
    }
    for code in [109999, 120000] {
        let outcome = parse_search_response(
            Err(Error::Rejected {
                status: 403,
                code,
                message: "Rejected".to_owned(),
            }),
            SearchScope::Channel(10),
        )
        .await;
        assert!(matches!(
            outcome,
            Err(Error::Rejected { status: 403, code: actual, message })
                if actual == code && message == "Rejected"
        ));
    }
    assert!(matches!(
        parse_search_response(Err(Error::Http(401)), SearchScope::Channel(10)).await,
        Err(Error::Http(401))
    ));
    assert!(matches!(
        parse_search_response(Err(Error::Network), SearchScope::Channel(10)).await,
        Err(Error::Network)
    ));
    assert!(matches!(
        parse_search_response(
            Err(Error::RateLimited {
                retry_after: Duration::from_secs(7),
            }),
            SearchScope::Channel(10),
        )
        .await,
        Err(Error::RateLimited { retry_after }) if retry_after == Duration::from_secs(7)
    ));
}

#[tokio::test]
async fn search_response_decodes_wire_bodies_and_tolerates_any_202() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::time::timeout;

    enum Expected {
        Indexing(Duration),
        Page,
        Decode,
    }

    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let cases = [
        (
            202,
            r#"{"code":110000,"retry_after":1.5}"#,
            Expected::Indexing(Duration::from_millis(1500)),
        ),
        (202, "", Expected::Indexing(Duration::from_secs(2))),
        (202, "not json", Expected::Indexing(Duration::from_secs(2))),
        (
            200,
            r#"{"code":119999,"retry_after":1.5}"#,
            Expected::Indexing(Duration::from_millis(1500)),
        ),
        (
            200,
            r#"{"total_results":42,"messages":[[]]}"#,
            Expected::Page,
        ),
        (200, "not json", Expected::Decode),
    ]
    .map(|(status, body, expected)| (SearchScope::Channel(10), status, "", body, expected));
    for (scope, status, headers, body, expected) in cases.into_iter().chain([
        (
            SearchScope::Dms,
            202,
            "Retry-After: 4.5\r\n",
            "",
            Expected::Indexing(Duration::from_millis(4500)),
        ),
        (
            SearchScope::Dms,
            202,
            "Retry-After: invalid\r\n",
            r#"{"retry_after":1.5}"#,
            Expected::Indexing(Duration::from_millis(1500)),
        ),
        (
            SearchScope::Dms,
            202,
            "Retry-After: 0\r\n",
            "not json",
            Expected::Indexing(Duration::from_secs(1)),
        ),
        (
            SearchScope::Dms,
            202,
            "Retry-After: 100\r\n",
            "",
            Expected::Indexing(Duration::from_secs(30)),
        ),
        (
            SearchScope::Dms,
            200,
            "",
            r#"{"tabs":{"messages":{"total_results":42,"messages":[[]],"cursor":{}}}}"#,
            Expected::Page,
        ),
        (
            SearchScope::Dms,
            200,
            "",
            r#"{"total_results":42,"messages":[[]]}"#,
            Expected::Decode,
        ),
    ]) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/search", listener.local_addr().unwrap());
        let server = async {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut bytes = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = stream.read(&mut bytes).await.unwrap();
                assert!(count > 0 && request.len() < 8192);
                request.extend_from_slice(&bytes[..count]);
            }
            stream.write_all(format!(
                "HTTP/1.1 {status} OK\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            ).as_bytes()).await.unwrap();
            stream.shutdown().await.unwrap();
        };
        let ((), response) = timeout(Duration::from_secs(3), async {
            tokio::join!(server, client.get(&url).send())
        })
        .await
        .unwrap();
        let outcome = parse_search_response(Ok(response.unwrap()), scope).await;
        match expected {
            Expected::Indexing(delay) => assert!(matches!(
                outcome,
                Ok(SearchOutcome::Indexing { retry_after }) if retry_after == delay
            )),
            Expected::Decode => assert!(matches!(outcome, Err(Error::Decode))),
            Expected::Page => {
                let Ok(SearchOutcome::Page(page)) = outcome else {
                    panic!("successful search must return a page");
                };
                assert_eq!(page.total, 42);
                assert!(page.messages.is_empty());
            }
        }
    }
}
