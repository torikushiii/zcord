use super::activity::rebuild;
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub enum ClientTheme {
    Preset(u32),
    Custom(CustomTheme),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CustomTheme {
    pub colors: Vec<String>,
    pub stops: Vec<f32>,
    pub angle: i32,
    pub base_mix: i32,
}

impl Default for CustomTheme {
    fn default() -> Self {
        Self {
            colors: Vec::new(),
            stops: Vec::new(),
            angle: 0,
            base_mix: 74,
        }
    }
}

fn custom_theme(bytes: &[u8]) -> Result<CustomTheme> {
    let mut theme = CustomTheme::default();
    let mut reader = Reader::new(bytes);
    while let Some((field, wire)) = reader.tag()? {
        match (field, wire) {
            (1, 2) => theme.colors.push(
                std::str::from_utf8(reader.bytes(wire)?)
                    .map_err(|_| DecodeError)?
                    .to_owned(),
            ),
            (2, 5) => {
                theme.stops.push(f32::from_le_bytes(
                    reader.take(4)?.try_into().map_err(|_| DecodeError)?,
                ));
            }
            (2, 2) => {
                let bytes = reader.bytes(wire)?;
                if bytes.len() % 4 != 0 {
                    return Err(DecodeError);
                }
                for bytes in bytes.as_chunks::<4>().0 {
                    theme.stops.push(f32::from_le_bytes(*bytes));
                }
            }
            (3, 0) => theme.angle = reader.varint()? as i32,
            (4, 0) => theme.base_mix = reader.varint()? as i32,
            (1..=4, _) => return Err(DecodeError),
            _ => reader.skip(field, wire, 0)?,
        }
    }
    Ok(theme)
}

fn client_theme_settings(bytes: &[u8]) -> Result<Option<ClientTheme>> {
    let mut preset = None;
    let mut custom = None;
    for field in fields(bytes)? {
        match field.number {
            2 => {
                preset = Some(
                    u32::try_from(number_wrapper(field.value.ok_or(DecodeError)?)?)
                        .map_err(|_| DecodeError)?,
                )
            }
            4 => custom = Some(custom_theme(field.value.ok_or(DecodeError)?)?),
            _ => {}
        }
    }
    Ok(custom
        .map(ClientTheme::Custom)
        .or(preset.map(ClientTheme::Preset)))
}

pub(super) fn decode_message(bytes: &[u8]) -> Result<(Option<AccountTheme>, Option<ClientTheme>)> {
    let mut theme = None;
    let mut client_theme = None;
    for field in fields(bytes)? {
        match field.number {
            1 if field.wire == 0 => {
                let mut reader = Reader::new(field.raw);
                reader.tag()?;
                theme = AccountTheme::from_wire(reader.varint()?);
            }
            3 => client_theme = client_theme_settings(field.value.ok_or(DecodeError)?)?,
            _ => {}
        }
    }
    Ok((theme, client_theme))
}

pub fn client_theme(preloaded: &[u8]) -> Result<Option<ClientTheme>> {
    let mut theme = None;
    for field in fields(preloaded)? {
        if field.number == 13 {
            theme = decode_message(field.value.ok_or(DecodeError)?)?.1;
        }
    }
    Ok(theme)
}

pub fn appearance_patch(
    preloaded: &[u8],
    theme: AccountTheme,
    client_theme: Option<&ClientTheme>,
) -> Result<Vec<u8>> {
    let mut encoded = Vec::new();
    push_uint(&mut encoded, 1, theme.wire());
    if let Some(theme) = client_theme {
        let mut client = Vec::new();
        let mut custom = Vec::new();
        for top in fields(preloaded)? {
            if top.number != 13 {
                continue;
            }
            for sub in fields(top.value.ok_or(DecodeError)?)? {
                if sub.number != 3 {
                    continue;
                }
                for field in fields(sub.value.ok_or(DecodeError)?)? {
                    match field.number {
                        2 => {}
                        4 if matches!(theme, ClientTheme::Custom(_)) => {
                            for field in fields(field.value.ok_or(DecodeError)?)? {
                                if !matches!(field.number, 1..=4) {
                                    custom.extend_from_slice(field.raw);
                                }
                            }
                        }
                        4 => {}
                        _ => client.extend_from_slice(field.raw),
                    }
                }
            }
        }
        match theme {
            ClientTheme::Preset(id) => {
                let mut wrapper = Vec::new();
                push_uint(&mut wrapper, 1, u64::from(*id));
                push_bytes(&mut client, 2, &wrapper);
            }
            ClientTheme::Custom(theme) => {
                for color in &theme.colors {
                    push_bytes(&mut custom, 1, color.as_bytes());
                }
                if !theme.stops.is_empty() {
                    let packed: Vec<u8> = theme
                        .stops
                        .iter()
                        .flat_map(|stop| stop.to_le_bytes())
                        .collect();
                    push_bytes(&mut custom, 2, &packed);
                }
                push_uint(&mut custom, 3, i64::from(theme.angle) as u64);
                push_uint(&mut custom, 4, i64::from(theme.base_mix) as u64);
                push_bytes(&mut client, 4, &custom);
            }
        }
        push_bytes(&mut encoded, 3, &client);
    }
    rebuild(preloaded, 13, &[1, 3], &encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_fixtures::{field, uint};

    #[test]
    fn defaults_preset_and_custom_precedence() {
        assert_eq!(client_theme(&[]).unwrap(), None);
        assert_eq!(client_theme(&field(13, &field(3, &[]))).unwrap(), None);
        let preset = field(2, &uint(1, 42));
        assert_eq!(
            client_theme(&field(13, &field(3, &preset))).unwrap(),
            Some(ClientTheme::Preset(42))
        );
        let custom = field(4, &[]);
        for client in [
            [preset.clone(), custom.clone()].concat(),
            [custom, preset].concat(),
        ] {
            assert_eq!(
                client_theme(&field(13, &field(3, &client))).unwrap(),
                Some(ClientTheme::Custom(CustomTheme::default()))
            );
        }
    }

    #[test]
    fn custom_decodes_packed_and_unpacked_stops_and_signed_ints() {
        let mut custom = [field(1, b"#5348ca"), field(1, b"#ffffff")].concat();
        custom.extend(field(
            2,
            &[0.0f32.to_le_bytes(), 0.5f32.to_le_bytes()].concat(),
        ));
        push_varint(&mut custom, (2 << 3) | 5);
        custom.extend(1.0f32.to_le_bytes());
        custom.extend(uint(3, -45i64 as u64));
        custom.extend(uint(4, 0));
        let proto = field(13, &field(3, &field(4, &custom)));
        assert_eq!(
            decode(&proto).unwrap().client_theme,
            Some(ClientTheme::Custom(CustomTheme {
                colors: vec!["#5348ca".into(), "#ffffff".into()],
                stops: vec![0.0, 0.5, 1.0],
                angle: -45,
                base_mix: 0,
            }))
        );
    }

    #[test]
    fn appearance_patch_preserves_opaque_siblings_and_switches_exclusive_modes() {
        let opaque = field(99, b"future-appearance");
        let client_opaque = uint(9, 77);
        let custom_opaque = field(10, b"future-custom");
        let base = [
            field(1, &uint(3, 9)),
            field(
                13,
                &[
                    uint(1, 1),
                    opaque.clone(),
                    field(
                        3,
                        &[
                            client_opaque.clone(),
                            field(2, &uint(1, 8)),
                            field(4, &custom_opaque),
                        ]
                        .concat(),
                    ),
                ]
                .concat(),
            ),
        ]
        .concat();
        for client in [
            None,
            Some(ClientTheme::Preset(42)),
            Some(ClientTheme::Custom(CustomTheme {
                colors: vec!["#5348ca".into(), "#ffffff".into()],
                stops: vec![0.0, 1.0],
                angle: -90,
                base_mix: 74,
            })),
        ] {
            let patch = appearance_patch(&base, AccountTheme::Midnight, client.as_ref()).unwrap();
            let top = fields(&patch).unwrap();
            assert_eq!(top.len(), 1);
            assert_eq!(top[0].number, 13);
            let content = fields(top[0].value.unwrap()).unwrap();
            assert!(content.iter().any(|f| f.raw == opaque));
            let decoded = decode(&patch).unwrap();
            assert_eq!(decoded.theme, Some(AccountTheme::Midnight));
            assert_eq!(decoded.client_theme, client);
            if client.is_some() {
                let client_bytes = content
                    .iter()
                    .find(|f| f.number == 3)
                    .unwrap()
                    .value
                    .unwrap();
                let fields = fields(client_bytes).unwrap();
                assert!(fields.iter().any(|f| f.raw == client_opaque));
                assert_eq!(
                    fields.iter().filter(|f| matches!(f.number, 2 | 4)).count(),
                    1
                );
            }
            let merged = merge_partial(&base, &patch).unwrap();
            assert_eq!(data_version(&merged).unwrap(), Some(9));
        }
        let patch = theme_patch(&base, AccountTheme::Light).unwrap();
        let before = fields(&base).unwrap();
        let before = fields(
            before
                .iter()
                .find(|f| f.number == 13)
                .unwrap()
                .value
                .unwrap(),
        )
        .unwrap();
        let after = fields(&patch).unwrap();
        let after = fields(after[0].value.unwrap()).unwrap();
        assert_eq!(
            before.iter().find(|f| f.number == 3).unwrap().raw,
            after.iter().find(|f| f.number == 3).unwrap().raw
        );
    }

    #[test]
    fn malformed_custom_values_fail_closed() {
        for custom in [field(2, &[0, 1]), uint(2, 1), field(1, &[255])] {
            assert!(client_theme(&field(13, &field(3, &field(4, &custom)))).is_err());
        }
    }
}
