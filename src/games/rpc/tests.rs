use super::*;

fn handshake(body: &str) -> Frame {
    Frame {
        opcode: OP_HANDSHAKE,
        payload: body.as_bytes().to_vec(),
    }
}

#[test]
fn frame_headers_round_trip() {
    let header = frame_header(OP_FRAME, 5).expect("frame fits");
    assert_eq!(parse_header(header).expect("header parses"), (OP_FRAME, 5));
}

#[test]
fn oversize_frames_are_rejected() {
    assert!(frame_header(OP_FRAME, MAX_FRAME).is_ok());
    assert!(frame_header(OP_FRAME, MAX_FRAME + 1).is_err());
    let mut header = [0; 8];
    header[..4].copy_from_slice(&OP_FRAME.to_le_bytes());
    header[4..].copy_from_slice(&((MAX_FRAME + 1) as u32).to_le_bytes());
    assert!(parse_header(header).is_err());
}

#[tokio::test]
async fn reading_an_oversize_frame_fails_before_reading_it() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&OP_FRAME.to_le_bytes());
    bytes.extend_from_slice(&((MAX_FRAME + 1) as u32).to_le_bytes());
    let mut input = bytes.as_slice();
    assert!(read_frame(&mut input).await.is_err());
}

#[test]
fn handshakes_require_version_one_and_a_numeric_client() {
    assert_eq!(
        parse_handshake(&handshake(r#"{"v":1,"client_id":"42"}"#)),
        Some(42)
    );
    assert_eq!(
        parse_handshake(&handshake(r#"{"v":1,"client_id":42}"#)),
        Some(42)
    );
    assert_eq!(
        parse_handshake(&handshake(r#"{"v":2,"client_id":"42"}"#)),
        None
    );
    assert_eq!(
        parse_handshake(&handshake(r#"{"v":1,"client_id":"x"}"#)),
        None
    );
    assert_eq!(parse_handshake(&handshake(r#"{"v":1}"#)), None);
    assert_eq!(
        parse_handshake(&Frame {
            opcode: OP_FRAME,
            payload: br#"{"v":1,"client_id":"42"}"#.to_vec(),
        }),
        None
    );
}

#[test]
fn activities_keep_only_allowed_fields() {
    let input = json!({
        "name": "Name",
        "type": 2,
        "details": "Details",
        "state": "State",
        "application_id": "999",
        "id": "spoof",
        "secrets": {"join": "secret"},
        "timestamps": {
            "start": 1_700_000_000_u64,
            "end": 1_700_000_030_000_u64,
            "extra": 30
        },
        "assets": {
            "large_image": "key",
            "large_text": "large",
            "small_image": "small",
            "small_text": "small text",
            "large_url": "https://example.com"
        },
        "party": {"id": "party", "size": [1, 2]},
        "buttons": [
            {"label": "One", "url": "https://example.com"},
            {"label": "Two", "url": "http://example.com"},
            {"label": "Three", "url": "https://example.com"}
        ],
        "metadata": {"button_urls": ["https://spoof.example.com"]}
    });
    let activity = build_activity(&input, 42).expect("activity is valid");
    assert_eq!(activity["application_id"], "42");
    assert_eq!(activity["type"], 2);
    assert_eq!(activity["name"], "Name");
    assert_eq!(activity["details"], "Details");
    assert_eq!(activity["state"], "State");
    assert!(activity.get("id").is_none());
    assert!(activity.get("secrets").is_none());
    assert_eq!(
        activity["timestamps"],
        json!({"start": 1_700_000_000_000_u64, "end": 1_700_000_030_000_u64})
    );
    assert_eq!(activity["assets"]["large_image"], "key");
    assert!(activity["assets"].get("large_url").is_none());
    assert_eq!(activity["party"], json!({"id": "party", "size": [1, 2]}));
    assert_eq!(activity["buttons"], json!(["One", "Two"]));
    assert_eq!(
        activity["metadata"],
        json!({"button_urls": ["https://example.com", "http://example.com"]})
    );
}

#[test]
fn timestamps_convert_seconds_and_keep_milliseconds() {
    let seconds = build_activity(
        &json!({"timestamps": {"start": 1_700_000_000_u64, "end": 1_700_000_030_u64}}),
        1,
    )
    .unwrap();
    assert_eq!(
        seconds["timestamps"],
        json!({"start": 1_700_000_000_000_u64, "end": 1_700_000_030_000_u64})
    );
    let millis = build_activity(
        &json!({"timestamps": {"start": 1_700_000_000_000_u64, "end": 1_700_000_030_000_u64}}),
        1,
    )
    .unwrap();
    assert_eq!(
        millis["timestamps"],
        json!({"start": 1_700_000_000_000_u64, "end": 1_700_000_030_000_u64})
    );
}

#[test]
fn timestamps_drop_zero_out_of_range_and_an_end_not_after_start() {
    let zero = build_activity(&json!({"timestamps": {"start": 0_u64, "end": 0_u64}}), 1).unwrap();
    assert!(zero.get("timestamps").is_none());

    let end_only = build_activity(
        &json!({"timestamps": {"start": 0_u64, "end": 1_700_000_000_u64}}),
        1,
    )
    .unwrap();
    assert_eq!(
        end_only["timestamps"],
        json!({"end": 1_700_000_000_000_u64})
    );

    let at_limit = build_activity(&json!({"timestamps": {"start": MAX_TIMESTAMP_MS}}), 1).unwrap();
    assert_eq!(at_limit["timestamps"], json!({"start": MAX_TIMESTAMP_MS}));
    let past_limit =
        build_activity(&json!({"timestamps": {"start": MAX_TIMESTAMP_MS + 1}}), 1).unwrap();
    assert!(past_limit.get("timestamps").is_none());
    let past_limit_from_seconds =
        build_activity(&json!({"timestamps": {"start": 4_102_444_801_u64}}), 1).unwrap();
    assert!(past_limit_from_seconds.get("timestamps").is_none());

    let inverted = build_activity(
        &json!({"timestamps": {"start": 1_700_000_030_u64, "end": 1_700_000_000_u64}}),
        1,
    )
    .unwrap();
    assert_eq!(
        inverted["timestamps"],
        json!({"start": 1_700_000_030_000_u64})
    );
    let equal = build_activity(
        &json!({"timestamps": {"start": 1_700_000_030_u64, "end": 1_700_000_030_u64}}),
        1,
    )
    .unwrap();
    assert_eq!(equal["timestamps"], json!({"start": 1_700_000_030_000_u64}));
}

#[test]
fn activity_types_default_and_buttons_require_http_urls() {
    assert_eq!(build_activity(&json!({}), 1).unwrap()["type"], 0);
    assert_eq!(build_activity(&json!({"type": 4}), 1).unwrap()["type"], 0);
    assert_eq!(build_activity(&json!({"type": 5}), 1).unwrap()["type"], 5);
    let activity = build_activity(
        &json!({"buttons": [
            {"label": "Bad", "url": "javascript:alert(1)"},
            {"label": "Good", "url": "https://example.com"}
        ]}),
        1,
    )
    .unwrap();
    assert_eq!(activity["buttons"], json!(["Good"]));
    assert_eq!(
        activity["metadata"]["button_urls"],
        json!(["https://example.com"])
    );
    let activity = build_activity(
        &json!({"buttons": [{"label": "Bad", "url": "ftp://example.com"}]}),
        1,
    )
    .unwrap();
    assert!(activity.get("buttons").is_none());
    assert!(activity.get("metadata").is_none());
}

#[test]
fn long_strings_are_truncated_on_char_boundaries() {
    let activity = build_activity(
        &json!({
            "details": "é".repeat(200),
            "assets": {"large_image": format!("{}é", "a".repeat(MAX_ASSET - 1))}
        }),
        1,
    )
    .unwrap();
    assert_eq!(activity["details"].as_str().unwrap().chars().count(), 128);
    assert_eq!(
        activity["assets"]["large_image"].as_str().unwrap().len(),
        MAX_ASSET - 1
    );
    assert_eq!(
        truncate_bytes(&"a".repeat(MAX_ASSET + 1), MAX_ASSET).len(),
        MAX_ASSET
    );
    assert_eq!(
        truncate_bytes(&format!("{}é", "a".repeat(MAX_ASSET - 1)), MAX_ASSET).len(),
        MAX_ASSET - 1
    );
}

#[test]
fn unsupported_commands_keep_the_activity() {
    let (update, reply) = handle_command(&json!({"cmd": "SUBSCRIBE", "nonce": "n"}), 42);
    assert!(matches!(update, Update::Keep));
    assert_eq!(reply["cmd"], "SUBSCRIBE");
    assert_eq!(reply["nonce"], "n");
    assert_eq!(reply["evt"], "ERROR");
    assert_eq!(reply["data"]["code"], 4000);
    assert_eq!(reply["data"]["message"], "Unsupported");
}

#[test]
fn set_activity_replies_and_null_clears() {
    let (update, reply) = handle_command(
        &json!({"cmd": "SET_ACTIVITY", "nonce": "n", "args": {"activity": {"name": "Game"}}}),
        42,
    );
    assert!(matches!(update, Update::Set(_)));
    assert_eq!(reply["cmd"], "SET_ACTIVITY");
    assert_eq!(reply["nonce"], "n");
    assert_eq!(reply["evt"], Value::Null);
    assert_eq!(reply["data"]["name"], "Game");
    assert_eq!(reply["data"]["application_id"], "42");
    let (update, reply) = handle_command(
        &json!({"cmd": "SET_ACTIVITY", "nonce": "n", "args": {"activity": null}}),
        42,
    );
    assert!(matches!(update, Update::Clear));
    assert_eq!(reply["data"], Value::Null);
    let (update, _) = handle_command(
        &json!({"cmd": "SET_ACTIVITY", "nonce": "n", "args": {}}),
        42,
    );
    assert!(matches!(update, Update::Clear));
}

#[test]
fn ready_matches_the_rpc_handshake_shape() {
    let ready = ready();
    assert_eq!(ready["cmd"], "DISPATCH");
    assert_eq!(ready["evt"], "READY");
    assert_eq!(ready["nonce"], Value::Null);
    assert_eq!(ready["data"]["v"], 1);
    assert_eq!(ready["data"]["config"]["api_endpoint"], "//discord.com/api");
    assert_eq!(ready["data"]["user"]["id"], "0");
    assert_eq!(ready["data"]["user"]["username"], "zcord");
}
