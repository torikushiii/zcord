//! Only the guild-folder subset of PreloadedUserSettings is decoded. Schema:
//! https://github.com/discord-userdoccers/discord-protos/blob/master/discord_protos/discord_users/v1/PreloadedUserSettings.proto
use base64::{Engine, engine::general_purpose::STANDARD};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuildFolder {
    pub guild_ids: Vec<u64>,
    pub id: Option<u64>,
    pub name: Option<String>,
    pub color: Option<u32>,
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

pub fn decode(bytes: &[u8]) -> Result<Option<Vec<GuildFolder>>> {
    let mut reader = Reader::new(bytes);
    let mut folders = None;
    while let Some((field, wire)) = reader.tag()? {
        if field == 14 {
            let mut guilds = Reader::new(reader.bytes(wire)?);
            let entries = folders.get_or_insert_with(Vec::new);
            while let Some((field, wire)) = guilds.tag()? {
                match field {
                    1 => entries.push(folder(guilds.bytes(wire)?)?),
                    // Deprecated guild_positions is validated but does not define rail order.
                    2 => fixed_ids(&mut guilds, wire, &mut Vec::new())?,
                    _ => guilds.skip(field, wire, 0)?,
                }
            }
        } else {
            reader.skip(field, wire, 0)?;
        }
    }
    Ok(folders)
}

pub fn decode_base64(encoded: &str) -> Result<Option<Vec<GuildFolder>>> {
    if encoded.len() > 8 * 1024 * 1024 {
        return Err(DecodeError);
    }
    decode(&STANDARD.decode(encoded).map_err(|_| DecodeError)?)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn varint(mut n: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        while n >= 128 {
            bytes.push((n as u8 & 127) | 128);
            n >>= 7;
        }
        bytes.push(n as u8);
        bytes
    }
    fn field(field: u32, bytes: &[u8]) -> Vec<u8> {
        let mut out = varint((u64::from(field) << 3) | 2);
        out.extend(varint(bytes.len() as u64));
        out.extend(bytes);
        out
    }
    pub(crate) fn fixture(packed: bool) -> String {
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
    #[test]
    fn folders_decode_packed_unpacked_and_wrappers() {
        for packed in [true, false] {
            let folders = decode_base64(&fixture(packed)).unwrap().unwrap();
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
        assert_eq!(decode(&[8, 0]).unwrap(), None);
        assert_eq!(decode(&[114, 0]).unwrap(), Some(Vec::new()));
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
}
