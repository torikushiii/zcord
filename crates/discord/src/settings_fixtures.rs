use base64::{Engine, engine::general_purpose::STANDARD};

pub fn varint(mut n: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    while n >= 128 {
        bytes.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    bytes.push(n as u8);
    bytes
}
pub fn field(field: u32, bytes: &[u8]) -> Vec<u8> {
    let mut out = varint((u64::from(field) << 3) | 2);
    out.extend(varint(bytes.len() as u64));
    out.extend(bytes);
    out
}
pub fn uint(field: u32, value: u64) -> Vec<u8> {
    let mut out = varint(u64::from(field) << 3);
    out.extend(varint(value));
    out
}
pub fn fixture(packed: bool) -> String {
    let mut single = vec![9];
    single.extend(100u64.to_le_bytes());
    let mut grouped = Vec::new();
    let ids = [200u64, 999];
    if packed {
        grouped.extend(field(
            1,
            &ids.into_iter()
                .flat_map(u64::to_le_bytes)
                .collect::<Vec<_>>(),
        ));
    } else {
        for id in ids {
            grouped.push(9);
            grouped.extend(id.to_le_bytes());
        }
    }
    grouped.extend(field(2, &[8, 42]));
    grouped.extend(field(3, &field(1, b"Gaming")));
    let mut color = vec![8];
    color.extend(varint(0x5865F2));
    grouped.extend(field(4, &color));
    let mut folders = field(1, &single);
    folders.extend(field(1, &grouped));
    folders.extend(field(2, &100u64.to_le_bytes()));
    let mut settings = vec![8, 1, 21, 0, 0, 0, 0, 25];
    settings.extend(0u64.to_le_bytes());
    settings.extend(field(14, &folders));
    settings.extend(field(20, b"ignored"));
    settings.extend([0xAB, 0x01, 8, 0, 0xAC, 0x01]);
    STANDARD.encode(settings)
}

pub fn status_fixture(status: Option<&str>, custom: Option<(&str, u64, &str, u64)>) -> String {
    let mut settings = Vec::new();
    if let Some(status) = status {
        settings.extend(field(1, &field(1, status.as_bytes())));
    }
    if let Some((text, emoji_id, emoji_name, expires)) = custom {
        let mut custom = field(1, text.as_bytes());
        custom.push(17);
        custom.extend(emoji_id.to_le_bytes());
        custom.extend(field(3, emoji_name.as_bytes()));
        custom.push(33);
        custom.extend(expires.to_le_bytes());
        settings.extend(field(2, &custom));
    }
    STANDARD.encode(field(11, &settings))
}
