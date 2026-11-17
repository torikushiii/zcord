use super::*;
use serde_json::json;

fn term(bytes: &[u8]) -> Result<Value, DecodeError> {
    let mut framed = vec![VERSION];
    framed.extend(bytes);
    decode(&framed)
}

fn binary(text: &str) -> Vec<u8> {
    let mut bytes = vec![BINARY];
    bytes.extend((text.len() as u32).to_be_bytes());
    bytes.extend(text.as_bytes());
    bytes
}

fn small_big(sign: u8, digits: &[u8]) -> Vec<u8> {
    let mut bytes = vec![SMALL_BIG, digits.len() as u8, sign];
    bytes.extend(digits);
    bytes
}

#[test]
fn decodes_scalars_atoms_and_binaries() {
    assert_eq!(term(&[SMALL_INTEGER, 200]), Ok(json!(200)));
    assert_eq!(term(&[INTEGER, 0xff, 0xff, 0xff, 0xfe]), Ok(json!(-2)));
    assert_eq!(
        term(&[INTEGER, 0x7f, 0xff, 0xff, 0xff]),
        Ok(json!(i32::MAX))
    );
    let mut float = vec![NEW_FLOAT];
    float.extend(1.5f64.to_be_bytes());
    assert_eq!(term(&float), Ok(json!(1.5)));
    let mut nan = vec![NEW_FLOAT];
    nan.extend(f64::NAN.to_be_bytes());
    assert_eq!(term(&nan), Err(DecodeError));
    for (bytes, expected) in [
        (&[SMALL_ATOM, 3, b'n', b'i', b'l'][..], json!(null)),
        (&[SMALL_ATOM_UTF8, 4, b'n', b'u', b'l', b'l'], json!(null)),
        (&[SMALL_ATOM, 4, b't', b'r', b'u', b'e'], json!(true)),
        (&[ATOM, 0, 5, b'f', b'a', b'l', b's', b'e'], json!(false)),
        (&[ATOM_UTF8, 0, 2, b'o', b'k'], json!("ok")),
        (&[SMALL_ATOM_UTF8, 2, 0xc3, 0xa9], json!("é")),
    ] {
        assert_eq!(term(bytes), Ok(expected));
    }
    assert_eq!(term(&binary("héllo")), Ok(json!("héllo")));
    assert_eq!(term(&binary("")), Ok(json!("")));
    assert_eq!(
        term(&[BINARY, 0, 0, 0, 2, 0xff, b'a']),
        Ok(json!("\u{fffd}a"))
    );
}

#[test]
fn integers_keep_json_shapes_with_snowflakes_as_strings() {
    assert_eq!(term(&small_big(0, &[])), Ok(json!(0)));
    assert_eq!(term(&small_big(1, &[0, 0])), Ok(json!(0)));
    assert_eq!(term(&small_big(0, &[0, 0, 0, 0x80])), Ok(json!(1u64 << 31)));
    assert_eq!(
        term(&small_big(1, &[0, 0, 0, 0x80])),
        Ok(json!(-(1i64 << 31)))
    );
    let timestamp = 1_763_400_000_000u64;
    assert_eq!(
        term(&small_big(0, &timestamp.to_le_bytes()[..6])),
        Ok(json!(timestamp))
    );
    assert_eq!(
        term(&small_big(0, &MAX_SAFE_INTEGER.to_le_bytes())),
        Ok(json!(MAX_SAFE_INTEGER))
    );
    assert_eq!(
        term(&small_big(0, &(MAX_SAFE_INTEGER + 1).to_le_bytes())),
        Ok(json!("9007199254740992"))
    );
    assert_eq!(
        term(&small_big(1, &(MAX_SAFE_INTEGER + 1).to_le_bytes())),
        Ok(json!("-9007199254740992"))
    );
    let snowflake = 1_175_185_049_000_000_000u64;
    assert_eq!(
        term(&small_big(0, &snowflake.to_le_bytes())),
        Ok(json!(snowflake.to_string()))
    );
    assert_eq!(
        term(&small_big(0, &u64::MAX.to_le_bytes())),
        Ok(json!(u64::MAX.to_string()))
    );
    let mut padded = snowflake.to_le_bytes().to_vec();
    padded.extend([0; 8]);
    assert_eq!(
        term(&small_big(0, &padded)),
        Ok(json!(snowflake.to_string()))
    );
    let mut large = vec![LARGE_BIG, 0, 0, 0, 16, 0];
    large.extend(u128::MAX.to_le_bytes());
    assert_eq!(term(&large), Ok(json!(u128::MAX.to_string())));
    assert_eq!(term(&small_big(0, &[1; 17])), Err(DecodeError));
    assert_eq!(term(&small_big(2, &[1])), Err(DecodeError));
}

#[test]
fn decodes_lists_tuples_strings_and_nil() {
    assert_eq!(term(&[NIL]), Ok(json!([])));
    assert_eq!(term(&[STRING, 0, 2, 0, 99]), Ok(json!([0, 99])));
    assert_eq!(
        term(&[LIST, 0, 0, 0, 2, SMALL_INTEGER, 1, SMALL_INTEGER, 2, NIL]),
        Ok(json!([1, 2]))
    );
    assert_eq!(
        term(&[LIST, 0, 0, 0, 1, SMALL_INTEGER, 1, SMALL_INTEGER, 2]),
        Ok(json!([1, 2]))
    );
    assert_eq!(
        term(&[SMALL_TUPLE, 2, SMALL_INTEGER, 1, NIL]),
        Ok(json!([1, []]))
    );
    assert_eq!(
        term(&[LARGE_TUPLE, 0, 0, 0, 1, SMALL_INTEGER, 7]),
        Ok(json!([7]))
    );
}

#[test]
fn map_keys_become_strings_whatever_their_term_type() {
    let mut bytes = vec![MAP, 0, 0, 0, 5];
    bytes.extend(binary("op"));
    bytes.extend([SMALL_INTEGER, 0]);
    bytes.extend([SMALL_ATOM, 1, b't']);
    bytes.extend(binary("READY"));
    bytes.extend(small_big(0, &1_175_185_049_000_000_000u64.to_le_bytes()));
    bytes.extend([SMALL_INTEGER, 3]);
    bytes.extend([SMALL_INTEGER, 5]);
    bytes.extend([SMALL_ATOM, 3, b'n', b'i', b'l']);
    bytes.extend([SMALL_ATOM, 3, b'n', b'i', b'l']);
    bytes.extend([NIL]);
    assert_eq!(
        term(&bytes),
        Ok(json!({
            "op": 0, "t": "READY", "1175185049000000000": 3, "5": null, "null": []
        }))
    );
    let mut nested_key = vec![MAP, 0, 0, 0, 1, NIL];
    nested_key.extend([SMALL_INTEGER, 1]);
    assert_eq!(term(&nested_key), Err(DecodeError));
}

#[test]
fn rejects_malformed_truncated_and_unsupported_terms() {
    assert_eq!(decode(&[]), Err(DecodeError));
    assert_eq!(decode(&[130, NIL]), Err(DecodeError));
    assert_eq!(term(&[NIL, NIL]), Err(DecodeError));
    assert_eq!(term(&[BINARY, 0, 0, 0, 5, b'a']), Err(DecodeError));
    assert_eq!(term(&[BINARY, 0xff, 0xff, 0xff, 0xff]), Err(DecodeError));
    assert_eq!(term(&[LIST, 0xff, 0xff, 0xff, 0xff, NIL]), Err(DecodeError));
    assert_eq!(term(&[MAP, 0xff, 0xff, 0xff, 0xff]), Err(DecodeError));
    assert_eq!(term(&[INTEGER, 0, 0]), Err(DecodeError));
    assert_eq!(term(&[80, 0, 0, 0, 1, 0x78, 0x9c]), Err(DecodeError));
    assert_eq!(term(&[77, 0, 0, 0, 1, 8, 0xff]), Err(DecodeError));
    assert_eq!(term(&[99]), Err(DecodeError));
    let mut deep = [SMALL_TUPLE, 1].repeat(MAX_DEPTH + 1);
    deep.push(NIL);
    assert_eq!(term(&deep), Err(DecodeError));
    let mut shallow = [SMALL_TUPLE, 1].repeat(MAX_DEPTH);
    shallow.push(NIL);
    assert!(term(&shallow).is_ok());
}

#[test]
fn encodes_like_the_desktop_packer() {
    assert_eq!(
        encode(&json!({"d": {"qos": null, "seq": 0}, "op": 40})),
        [
            &[VERSION, MAP, 0, 0, 0, 2][..],
            &binary("d"),
            &[MAP, 0, 0, 0, 2],
            &binary("qos"),
            &[SMALL_ATOM, 3, b'n', b'i', b'l'],
            &binary("seq"),
            &[SMALL_INTEGER, 0],
            &binary("op"),
            &[SMALL_INTEGER, 40],
        ]
        .concat()
    );
    let encoded = |value: Value| encode(&value)[1..].to_vec();
    assert_eq!(
        encoded(json!(true)),
        [SMALL_ATOM, 4, b't', b'r', b'u', b'e']
    );
    assert_eq!(
        encoded(json!(false)),
        [SMALL_ATOM, 5, b'f', b'a', b'l', b's', b'e']
    );
    assert_eq!(encoded(json!("id")), binary("id"));
    assert_eq!(encoded(json!([])), [NIL]);
    assert_eq!(
        encoded(json!(["a", 1])),
        [
            &[LIST, 0, 0, 0, 2][..],
            &binary("a"),
            &[SMALL_INTEGER, 1, NIL]
        ]
        .concat()
    );
    assert_eq!(encoded(json!(255)), [SMALL_INTEGER, 255]);
    assert_eq!(encoded(json!(256)), [INTEGER, 0, 0, 1, 0]);
    assert_eq!(encoded(json!(-1)), [INTEGER, 0xff, 0xff, 0xff, 0xff]);
    assert_eq!(encoded(json!(i32::MAX)), [INTEGER, 0x7f, 0xff, 0xff, 0xff]);
    assert_eq!(encoded(json!(i32::MIN)), [SMALL_BIG, 4, 1, 0, 0, 0, 0x80]);
    assert_eq!(encoded(json!(1u64 << 31)), [SMALL_BIG, 4, 0, 0, 0, 0, 0x80]);
    assert_eq!(
        encoded(json!(1_763_400_000_000u64)),
        small_big(0, &1_763_400_000_000u64.to_le_bytes()[..6])
    );
    assert_eq!(encoded(json!(u64::MAX)), small_big(0, &[0xff; 8]));
    assert_eq!(encoded(json!(2.0)), [SMALL_INTEGER, 2]);
    assert_eq!(encoded(json!(-3.0)), [INTEGER, 0xff, 0xff, 0xff, 0xfd]);
    let mut float = vec![NEW_FLOAT];
    float.extend(0.25f64.to_be_bytes());
    assert_eq!(encoded(json!(0.25)), float);
}

#[test]
fn outgoing_payloads_round_trip() {
    let payload = json!({"op": 2, "d": {
        "token": "fixture-secret", "capabilities": 1_734_653,
        "properties": {"os": "Linux", "client_event_source": null, "client_build_number": 673_919},
        "presence": {"status": "unknown", "since": 0, "activities": [], "afk": false},
        "compress": false, "client_state": {"guild_versions": {}}
    }});
    assert_eq!(decode(&encode(&payload)), Ok(payload));
    let subscription = json!({"op": 37, "d": {"subscriptions": {"1175185049000000000": {
        "channels": {"1175185049000000001": [[0, 99], [100, 199]]}
    }}}});
    assert_eq!(decode(&encode(&subscription)), Ok(subscription));
}
