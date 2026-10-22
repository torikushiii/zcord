//! Guild folders and status from PreloadedUserSettings. Schema:
//! https://github.com/discord-userdoccers/discord-protos/blob/master/discord_protos/discord_users/v1/PreloadedUserSettings.proto
use base64::{Engine, engine::general_purpose::STANDARD};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuildFolder {
    pub guild_ids: Vec<u64>,
    pub id: Option<u64>,
    pub name: Option<String>,
    pub color: Option<u32>,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Settings {
    pub folders: Option<Vec<GuildFolder>>,
    pub status: Option<StatusSettings>,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct StatusSettings {
    pub status: Option<String>,
    pub custom_status: Option<CustomStatus>,
    pub custom_status_present: bool,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct CustomStatus {
    pub text: String,
    pub emoji_id: u64,
    pub emoji_name: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DecodeError;
type Result<T> = std::result::Result<T, DecodeError>;

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self.offset.checked_add(length).ok_or(DecodeError)?;
        let bytes = self.bytes.get(self.offset..end).ok_or(DecodeError)?;
        self.offset = end;
        Ok(bytes)
    }
    fn varint(&mut self) -> Result<u64> {
        let mut value = 0;
        for shift in (0..70).step_by(7) {
            let byte = self.take(1)?[0];
            if shift == 63 && byte > 1 {
                return Err(DecodeError);
            }
            value |= u64::from(byte & 127) << shift;
            if byte & 128 == 0 {
                return Ok(value);
            }
        }
        Err(DecodeError)
    }
    fn tag(&mut self) -> Result<Option<(u32, u8)>> {
        if self.offset == self.bytes.len() {
            return Ok(None);
        }
        let value = self.varint()?;
        let field = value >> 3;
        if field == 0 || field > 0x1fff_ffff {
            return Err(DecodeError);
        }
        Ok(Some((field as u32, (value & 7) as u8)))
    }
    fn bytes(&mut self, wire: u8) -> Result<&'a [u8]> {
        if wire != 2 {
            return Err(DecodeError);
        }
        let length = usize::try_from(self.varint()?).map_err(|_| DecodeError)?;
        self.take(length)
    }
    fn fixed64(&mut self) -> Result<u64> {
        let mut bytes = [0; 8];
        bytes.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(bytes))
    }
    fn skip(&mut self, field: u32, wire: u8, depth: usize) -> Result<()> {
        match wire {
            0 => {
                self.varint()?;
            }
            1 => {
                self.take(8)?;
            }
            2 => {
                self.bytes(wire)?;
            }
            3 if depth < 32 => loop {
                let (next, wire) = self.tag()?.ok_or(DecodeError)?;
                if wire == 4 {
                    if next != field {
                        return Err(DecodeError);
                    }
                    break;
                }
                self.skip(next, wire, depth + 1)?;
            },
            5 => {
                self.take(4)?;
            }
            _ => return Err(DecodeError),
        }
        Ok(())
    }
}

fn fixed_ids(reader: &mut Reader<'_>, wire: u8, ids: &mut Vec<u64>) -> Result<()> {
    match wire {
        1 => ids.push(reader.fixed64()?),
        2 => {
            let bytes = reader.bytes(wire)?;
            if bytes.len() % 8 != 0 {
                return Err(DecodeError);
            }
            let mut packed = Reader::new(bytes);
            while packed.offset < bytes.len() {
                ids.push(packed.fixed64()?);
            }
        }
        _ => return Err(DecodeError),
    }
    Ok(())
}

fn number_wrapper(bytes: &[u8]) -> Result<u64> {
    let mut reader = Reader::new(bytes);
    let mut value = 0;
    while let Some((field, wire)) = reader.tag()? {
        if field == 1 {
            if wire != 0 {
                return Err(DecodeError);
            }
            value = reader.varint()?;
        } else {
            reader.skip(field, wire, 0)?;
        }
    }
    Ok(value)
}

fn string_wrapper(bytes: &[u8]) -> Result<String> {
    let mut reader = Reader::new(bytes);
    let mut value = String::new();
    while let Some((field, wire)) = reader.tag()? {
        if field == 1 {
            value = std::str::from_utf8(reader.bytes(wire)?)
                .map_err(|_| DecodeError)?
                .to_owned();
        } else {
            reader.skip(field, wire, 0)?;
        }
    }
    Ok(value)
}

fn folder(bytes: &[u8]) -> Result<GuildFolder> {
    let mut reader = Reader::new(bytes);
    let mut folder = GuildFolder {
        guild_ids: Vec::new(),
        id: None,
        name: None,
        color: None,
    };
    while let Some((field, wire)) = reader.tag()? {
        match field {
            1 => fixed_ids(&mut reader, wire, &mut folder.guild_ids)?,
            2 => folder.id = Some(number_wrapper(reader.bytes(wire)?)?),
            3 => folder.name = Some(string_wrapper(reader.bytes(wire)?)?),
            4 => {
                folder.color = Some(
                    u32::try_from(number_wrapper(reader.bytes(wire)?)?).map_err(|_| DecodeError)?,
                )
            }
            _ => reader.skip(field, wire, 0)?,
        }
    }
    Ok(folder)
}

fn custom_status(bytes: &[u8]) -> Result<CustomStatus> {
    let mut reader = Reader::new(bytes);
    let mut status = CustomStatus::default();
    while let Some((field, wire)) = reader.tag()? {
        match (field, wire) {
            (1, 2) => {
                status.text = std::str::from_utf8(reader.bytes(wire)?)
                    .map_err(|_| DecodeError)?
                    .to_owned()
            }
            (2, 1) => status.emoji_id = reader.fixed64()?,
            (3, 2) => {
                status.emoji_name = std::str::from_utf8(reader.bytes(wire)?)
                    .map_err(|_| DecodeError)?
                    .to_owned()
            }
            (4, 1) => status.expires_at_ms = reader.fixed64()?,
            _ => reader.skip(field, wire, 0)?,
        }
    }
    Ok(status)
}

fn status_settings(bytes: &[u8]) -> Result<StatusSettings> {
    let mut reader = Reader::new(bytes);
    let mut status = StatusSettings::default();
    while let Some((field, wire)) = reader.tag()? {
        match field {
            1 => status.status = Some(string_wrapper(reader.bytes(wire)?)?),
            2 => {
                status.custom_status_present = true;
                let custom = custom_status(reader.bytes(wire)?)?;
                let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
                if (custom.expires_at_ms == 0 || custom.expires_at_ms >= now)
                    && (!custom.text.is_empty()
                        || !custom.emoji_name.is_empty()
                        || custom.emoji_id != 0)
                {
                    status.custom_status = Some(custom);
                }
            }
            _ => reader.skip(field, wire, 0)?,
        }
    }
    Ok(status)
}

pub fn decode(bytes: &[u8]) -> Result<Settings> {
    let mut reader = Reader::new(bytes);
    let mut settings = Settings::default();
    while let Some((field, wire)) = reader.tag()? {
        if field == 14 {
            let mut guilds = Reader::new(reader.bytes(wire)?);
            let entries = settings.folders.get_or_insert_with(Vec::new);
            while let Some((field, wire)) = guilds.tag()? {
                match field {
                    1 => entries.push(folder(guilds.bytes(wire)?)?),
                    // Deprecated guild_positions is validated but does not define rail order.
                    2 => fixed_ids(&mut guilds, wire, &mut Vec::new())?,
                    _ => guilds.skip(field, wire, 0)?,
                }
            }
        } else if field == 11 {
            settings.status = Some(status_settings(reader.bytes(wire)?)?);
        } else {
            reader.skip(field, wire, 0)?;
        }
    }
    Ok(settings)
}

pub fn decode_base64(encoded: &str) -> Result<Settings> {
    if encoded.len() > 8 * 1024 * 1024 {
        return Err(DecodeError);
    }
    decode(&STANDARD.decode(encoded).map_err(|_| DecodeError)?)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use crate::discord::settings_fixtures::fixture;

    #[test]
    fn folders_decode_packed_unpacked_and_wrappers() {
        for packed in [true, false] {
            let folders = decode_base64(&fixture(packed)).unwrap().folders.unwrap();
            assert_eq!(folders.len(), 2);
            assert_eq!(folders[0].guild_ids, [100]);
            assert_eq!(folders[0].id, None);
            assert_eq!(
                folders[1],
                GuildFolder {
                    guild_ids: vec![200, 999],
                    id: Some(42),
                    name: Some("Gaming".into()),
                    color: Some(0x5865F2)
                }
            );
        }
        assert_eq!(decode(&[8, 0]).unwrap().folders, None);
        assert_eq!(decode(&[114, 0]).unwrap().folders, Some(Vec::new()));
    }
    #[test]
    fn malformed_protobuf_is_rejected_without_panicking() {
        for bytes in [
            &[114, 10, 0][..],
            &[0xff; 11],
            &[0],
            &[14],
            &[11, 20],
            &[114, 3, 10, 1, 9],
        ] {
            assert!(decode(bytes).is_err());
        }
        let valid = STANDARD.decode(fixture(true)).unwrap();
        for length in 0..valid.len() {
            let _ = decode(&valid[..length]);
        }
        assert!(decode_base64("not base64!").is_err());
    }

    #[test]
    fn status_presence_wrappers_and_expiry() {
        use crate::discord::settings_fixtures::status_fixture;
        let decoded = decode_base64(&status_fixture(
            Some("invisible"),
            Some(("working", 0, "\u{1f3ae}", 0)),
        ))
        .unwrap();
        let status = decoded.status.unwrap();
        assert!(decoded.folders.is_none());
        assert_eq!(status.status.as_deref(), Some("invisible"));
        assert!(status.custom_status_present);
        assert_eq!(status.custom_status.unwrap().text, "working");
        let expired = decode_base64(&status_fixture(None, Some(("gone", 7, "x", 1))))
            .unwrap()
            .status
            .unwrap();
        assert_eq!(expired.status, None);
        assert!(expired.custom_status_present);
        assert!(expired.custom_status.is_none());
        let empty = decode_base64(&status_fixture(None, None))
            .unwrap()
            .status
            .unwrap();
        assert!(!empty.custom_status_present);
        assert!(decode(&[]).unwrap().status.is_none());
        let future = u64::MAX;
        assert!(
            decode_base64(&status_fixture(
                Some("idle"),
                Some(("future", 42, "custom", future))
            ))
            .unwrap()
            .status
            .unwrap()
            .custom_status
            .is_some()
        );
    }
}
