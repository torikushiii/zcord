//! Guild folders and status from PreloadedUserSettings, favorite GIFs from FrecencyUserSettings.
//! Schemas: https://github.com/discord-userdoccers/discord-protos/tree/master/discord_protos/discord_users/v1
use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::{HashMap, HashSet};

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

/// Manual presence timing, separate from CustomStatus.expires_at_ms.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct StatusTiming {
    pub expires_at_ms: u64,
    pub created_at_ms: Option<u64>,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct CustomStatus {
    pub text: String,
    pub emoji_id: u64,
    pub emoji_name: String,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FavoriteGif {
    pub url: String,
    pub src: String,
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub order: u32,
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

/// PATCH replaces the entire top-level status message. Preserve its opaque siblings
/// (custom status, activity sharing and future fields), but send no other settings.
pub fn status_patch(preloaded: &[u8], status: &str) -> Result<Vec<u8>> {
    status_patch_at(preloaded, status, 0, None)
}

/// Explicit selections replace manual timing. Unknown fields inside the status and
/// creation wrappers survive even when Forever/Online clear their known values.
pub fn status_patch_at(
    preloaded: &[u8],
    status: &str,
    expires_at_ms: u64,
    created_at_ms: Option<u64>,
) -> Result<Vec<u8>> {
    status_timing(preloaded)?;
    let (expires_at_ms, created_at_ms) = if status == "online" {
        (0, None)
    } else {
        (expires_at_ms, created_at_ms)
    };
    let mut content = Vec::new();
    let mut wrapper = Vec::new();
    let mut created_wrapper = Vec::new();
    for field in fields(preloaded)? {
        if field.number == 11 {
            for sub in fields(field.value.ok_or(DecodeError)?)? {
                match sub.number {
                    1 | 5 => {
                        for field in fields(sub.value.ok_or(DecodeError)?)? {
                            if field.number != 1 {
                                let output = if sub.number == 1 {
                                    &mut wrapper
                                } else {
                                    &mut created_wrapper
                                };
                                output.extend_from_slice(field.raw);
                            }
                        }
                    }
                    4 => {}
                    _ => content.extend_from_slice(sub.raw),
                }
            }
        }
    }
    push_bytes(&mut wrapper, 1, status.as_bytes());
    push_bytes(&mut content, 1, &wrapper);
    // fixed64, not a varint and not the nested custom-status expiration.
    push_varint(&mut content, (4 << 3) | 1);
    content.extend_from_slice(&expires_at_ms.to_le_bytes());
    if created_at_ms.is_some() || !created_wrapper.is_empty() {
        push_uint(&mut created_wrapper, 1, created_at_ms.unwrap_or(0));
        push_bytes(&mut content, 5, &created_wrapper);
    }
    let mut patch = Vec::new();
    push_bytes(&mut patch, 11, &content);
    Ok(patch)
}

pub fn status_timing(preloaded: &[u8]) -> Result<StatusTiming> {
    let mut timing = StatusTiming::default();
    for field in fields(preloaded)? {
        if field.number != 11 {
            continue;
        }
        let mut reader = Reader::new(field.value.ok_or(DecodeError)?);
        while let Some((field, wire)) = reader.tag()? {
            match field {
                4 if wire == 1 => timing.expires_at_ms = reader.fixed64()?,
                4 => return Err(DecodeError),
                5 => timing.created_at_ms = Some(number_wrapper(reader.bytes(wire)?)?),
                _ => reader.skip(field, wire, 0)?,
            }
        }
    }
    Ok(timing)
}

/// Required for a compare-and-swap PATCH so another client's edits cannot be lost.
pub fn data_version(preloaded: &[u8]) -> Result<Option<u64>> {
    let mut version = None;
    for field in fields(preloaded)? {
        if field.number != 1 {
            continue;
        }
        let mut reader = Reader::new(field.value.ok_or(DecodeError)?);
        version.get_or_insert(0);
        while let Some((field, wire)) = reader.tag()? {
            if field == 3 && wire == 0 {
                version = Some(reader.varint()?);
            } else {
                reader.skip(field, wire, 0)?;
            }
        }
    }
    Ok(version)
}

struct Field<'a> {
    number: u32,
    wire: u8,
    raw: &'a [u8],
    value: Option<&'a [u8]>,
}

fn fields(bytes: &[u8]) -> Result<Vec<Field<'_>>> {
    let mut reader = Reader::new(bytes);
    let mut fields = Vec::new();
    loop {
        let start = reader.offset;
        let Some((number, wire)) = reader.tag()? else {
            break;
        };
        let value = match wire {
            2 => Some(reader.bytes(wire)?),
            0 => {
                reader.varint()?;
                None
            }
            1 => {
                reader.take(8)?;
                None
            }
            5 => {
                reader.take(4)?;
                None
            }
            _ => {
                reader.skip(number, wire, 0)?;
                None
            }
        };
        fields.push(Field {
            number,
            wire,
            raw: &bytes[start..reader.offset],
            value,
        });
    }
    Ok(fields)
}

fn favorite_gif(bytes: &[u8]) -> Result<FavoriteGif> {
    let mut reader = Reader::new(bytes);
    let mut gif = FavoriteGif {
        url: String::new(),
        src: String::new(),
        format: 0,
        width: 0,
        height: 0,
        order: 0,
    };
    while let Some((field, wire)) = reader.tag()? {
        match (field, wire) {
            (1, 0) => gif.format = u32::try_from(reader.varint()?).map_err(|_| DecodeError)?,
            (2, 2) => {
                gif.src = std::str::from_utf8(reader.bytes(wire)?)
                    .map_err(|_| DecodeError)?
                    .to_owned()
            }
            (3, 0) => gif.width = u32::try_from(reader.varint()?).map_err(|_| DecodeError)?,
            (4, 0) => gif.height = u32::try_from(reader.varint()?).map_err(|_| DecodeError)?,
            (5, 0) => gif.order = u32::try_from(reader.varint()?).map_err(|_| DecodeError)?,
            _ => reader.skip(field, wire, 0)?,
        }
    }
    Ok(gif)
}

fn favorite_entry(bytes: &[u8]) -> Result<(String, FavoriteGif)> {
    let mut reader = Reader::new(bytes);
    let mut url = None;
    let mut gif = None;
    while let Some((field, wire)) = reader.tag()? {
        match (field, wire) {
            (1, 2) => {
                url = Some(
                    std::str::from_utf8(reader.bytes(wire)?)
                        .map_err(|_| DecodeError)?
                        .to_owned(),
                )
            }
            (2, 2) => gif = Some(favorite_gif(reader.bytes(wire)?)?),
            _ => reader.skip(field, wire, 0)?,
        }
    }
    let url = url.filter(|url| !url.is_empty()).ok_or(DecodeError)?;
    let mut gif = gif.unwrap_or(FavoriteGif {
        url: String::new(),
        src: String::new(),
        format: 0,
        width: 0,
        height: 0,
        order: 0,
    });
    gif.url = url.clone();
    Ok((url, gif))
}

pub fn favorite_gifs(frecency: &[u8]) -> Result<Vec<FavoriteGif>> {
    let mut gifs = HashMap::new();
    for field in fields(frecency)? {
        if field.number != 2 {
            continue;
        }
        for entry in fields(field.value.ok_or(DecodeError)?)? {
            if entry.number != 1 {
                continue;
            }
            let (url, gif) = favorite_entry(entry.value.ok_or(DecodeError)?)?;
            gifs.insert(url, gif);
        }
    }
    let mut gifs: Vec<FavoriteGif> = gifs.into_values().collect();
    gifs.sort_by(|left, right| {
        right
            .order
            .cmp(&left.order)
            .then_with(|| left.url.cmp(&right.url))
    });
    Ok(gifs)
}

fn push_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}

fn push_bytes(out: &mut Vec<u8>, field: u32, value: &[u8]) {
    push_varint(out, (u64::from(field) << 3) | 2);
    push_varint(out, value.len() as u64);
    out.extend_from_slice(value);
}

fn push_uint(out: &mut Vec<u8>, field: u32, value: u64) {
    push_varint(out, u64::from(field) << 3);
    push_varint(out, value);
}

fn favorite_entry_bytes(gif: &FavoriteGif) -> Vec<u8> {
    let mut value = Vec::new();
    push_uint(&mut value, 1, u64::from(gif.format));
    push_bytes(&mut value, 2, gif.src.as_bytes());
    push_uint(&mut value, 3, u64::from(gif.width));
    push_uint(&mut value, 4, u64::from(gif.height));
    push_uint(&mut value, 5, u64::from(gif.order));
    let mut entry = Vec::new();
    push_bytes(&mut entry, 1, gif.url.as_bytes());
    push_bytes(&mut entry, 2, &value);
    // Map entries are a repeated message field, not bare key/value pairs.
    let mut wrapped = Vec::new();
    push_bytes(&mut wrapped, 1, &entry);
    wrapped
}

pub fn with_favorite_gifs(frecency: &[u8], gifs: &[FavoriteGif]) -> Result<Vec<u8>> {
    let top = fields(frecency)?;
    let mut preserved = Vec::new();
    for field in &top {
        if field.number != 2 {
            continue;
        }
        let Some(value) = field.value else {
            return Err(DecodeError);
        };
        for sub in fields(value)? {
            if sub.number == 1 && sub.wire == 2 {
                continue;
            }
            preserved.push(sub.raw);
        }
    }
    let mut content = Vec::new();
    for raw in preserved {
        content.extend_from_slice(raw);
    }
    for gif in gifs {
        content.extend(favorite_entry_bytes(gif));
    }
    let mut out = Vec::with_capacity(frecency.len() + content.len());
    let mut written = false;
    for field in &top {
        if field.number == 2 {
            if !written {
                push_bytes(&mut out, 2, &content);
                written = true;
            }
        } else {
            out.extend_from_slice(field.raw);
        }
    }
    if !written {
        push_bytes(&mut out, 2, &content);
    }
    Ok(out)
}

pub fn merge_partial(base: &[u8], update: &[u8]) -> Result<Vec<u8>> {
    let base = fields(base)?;
    let update = fields(update)?;
    let numbers: HashSet<u32> = update.iter().map(|field| field.number).collect();
    let mut merged = Vec::with_capacity(base.len() + update.len());
    let mut replaced = HashSet::new();
    for field in &base {
        if numbers.contains(&field.number) {
            if replaced.insert(field.number) {
                for next in update.iter().filter(|next| next.number == field.number) {
                    merged.extend_from_slice(next.raw);
                }
            }
        } else {
            merged.extend_from_slice(field.raw);
        }
    }
    for field in &update {
        if !base.iter().any(|existing| existing.number == field.number) {
            merged.extend_from_slice(field.raw);
        }
    }
    Ok(merged)
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

    use crate::discord::settings_fixtures::{field, uint};

    fn fixed64_field(number: u32, value: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_varint(&mut bytes, (u64::from(number) << 3) | 1);
        bytes.extend_from_slice(&value.to_le_bytes());
        bytes
    }

    #[test]
    fn manual_status_timing_is_distinct_from_custom_status_expiration() {
        let mut custom = field(1, b"keep custom");
        custom.extend(fixed64_field(4, u64::MAX));
        let mut status = field(1, &field(1, b"idle"));
        status.extend(field(2, &custom));
        status.extend(fixed64_field(4, 9_000));
        status.extend(field(5, &uint(1, 1_000)));
        let proto = field(11, &status);
        assert_eq!(
            status_timing(&proto).unwrap(),
            StatusTiming {
                expires_at_ms: 9_000,
                created_at_ms: Some(1_000),
            }
        );
        assert_eq!(
            decode(&proto)
                .unwrap()
                .status
                .unwrap()
                .custom_status
                .unwrap()
                .expires_at_ms,
            u64::MAX
        );
        assert_eq!(status_timing(&[]).unwrap(), StatusTiming::default());
    }

    #[test]
    fn explicit_duration_replaces_manual_timing_and_preserves_unknown_wrapper_metadata() {
        let status_unknown = uint(77, 3);
        let created_unknown = field(88, b"creation-metadata");
        let mut wrapper = field(1, b"invisible");
        wrapper.extend(&status_unknown);
        let mut created = uint(1, 111);
        created.extend(&created_unknown);
        let mut custom = field(1, b"keep custom");
        custom.extend(fixed64_field(4, u64::MAX));
        let custom = field(2, &custom);
        let sharing = field(3, &uint(1, 0));
        let unknown = field(99, b"unknown-status-setting");
        let mut status = field(1, &wrapper);
        status.extend(&custom);
        status.extend(&sharing);
        status.extend(fixed64_field(4, 999));
        status.extend(field(5, &created));
        status.extend(&unknown);
        let mut base = field(11, &status);
        let unrelated = field(100, b"do not send or overwrite");
        base.extend(&unrelated);

        for (choice, expires, created, expected_expires, expected_created) in [
            ("dnd", 30_000, Some(2_000), 30_000, 2_000),
            ("idle", 0, None, 0, 0),
            ("online", 30_000, Some(2_000), 0, 0),
        ] {
            let patch = status_patch_at(&base, choice, expires, created).unwrap();
            let top = fields(&patch).unwrap();
            assert_eq!(top.len(), 1);
            assert_eq!(top[0].number, 11);
            let content = fields(top[0].value.unwrap()).unwrap();
            for preserved in [&custom, &sharing, &unknown] {
                assert!(
                    content
                        .iter()
                        .any(|field| field.raw == preserved.as_slice())
                );
            }
            for (number, opaque) in [(1, &status_unknown), (5, &created_unknown)] {
                let wrapper = content.iter().find(|field| field.number == number).unwrap();
                assert!(
                    fields(wrapper.value.unwrap())
                        .unwrap()
                        .iter()
                        .any(|field| field.raw == opaque.as_slice())
                );
            }
            let timing = status_timing(&patch).unwrap();
            assert_eq!(timing.expires_at_ms, expected_expires);
            assert_eq!(timing.created_at_ms, Some(expected_created));
            let expiry = content.iter().find(|field| field.number == 4).unwrap();
            assert_eq!(expiry.wire, 1, "manual expiry is fixed64");
            let merged = merge_partial(&base, &patch).unwrap();
            assert!(
                fields(&merged)
                    .unwrap()
                    .iter()
                    .any(|field| field.raw == unrelated)
            );
        }
        let forever = status_patch(&base, "invisible").unwrap();
        assert_eq!(status_timing(&forever).unwrap().expires_at_ms, 0);
    }

    #[test]
    fn malformed_manual_timing_fails_closed_without_touching_custom_timing() {
        for status in [uint(4, 123), field(5, &[8, 128]), vec![33, 1, 2]] {
            let proto = field(11, &status);
            assert!(status_timing(&proto).is_err());
            assert!(status_patch_at(&proto, "idle", 1_000, Some(1)).is_err());
        }
    }

    #[test]
    fn status_patch_changes_only_the_status_wrapper_and_preserves_opaque_siblings() {
        let mut wrapper = field(1, b"invisible");
        wrapper.extend(uint(88, 7));
        let custom = field(2, &field(1, b"working"));
        let sharing = field(3, &uint(1, 0));
        let unknown = field(99, b"future-status-field");
        let mut status = field(1, &wrapper);
        status.extend(&custom);
        status.extend(&sharing);
        status.extend(&unknown);
        let unrelated = field(90, b"untouched");
        let mut base = field(1, &uint(3, 42));
        base.extend(field(11, &status));
        base.extend(&unrelated);
        for status in ["online", "idle", "dnd", "invisible"] {
            let patch = status_patch(&base, status).unwrap();
            let top = fields(&patch).unwrap();
            assert_eq!(top.len(), 1);
            assert_eq!(top[0].number, 11);
            let content = fields(top[0].value.unwrap()).unwrap();
            for preserved in [&custom, &sharing, &unknown] {
                assert!(
                    content
                        .iter()
                        .any(|field| field.raw == preserved.as_slice())
                );
            }
            let wrapper = content.iter().find(|field| field.number == 1).unwrap();
            assert!(
                fields(wrapper.value.unwrap())
                    .unwrap()
                    .iter()
                    .any(|field| field.raw == uint(88, 7))
            );
            let merged = merge_partial(&base, &patch).unwrap();
            assert_eq!(
                decode(&merged).unwrap().status.unwrap().status.as_deref(),
                Some(status)
            );
            assert_eq!(data_version(&merged).unwrap(), Some(42));
            assert!(
                fields(&merged)
                    .unwrap()
                    .iter()
                    .any(|field| field.raw == unrelated)
            );
        }
    }

    #[test]
    fn status_patch_and_version_fail_closed_on_malformed_input() {
        assert!(status_patch(&[90, 9, 0], "online").is_err());
        assert!(data_version(&[10, 4, 24]).is_err());
        assert_eq!(data_version(&[]).unwrap(), None);
        assert_eq!(data_version(&field(1, &[])).unwrap(), Some(0));
        assert_eq!(
            decode(&status_patch(&[], "invisible").unwrap())
                .unwrap()
                .status
                .unwrap()
                .status
                .as_deref(),
            Some("invisible")
        );
    }

    fn favorite(url: &str, src: &str, order: u32) -> FavoriteGif {
        FavoriteGif {
            url: url.into(),
            src: src.into(),
            format: 1,
            width: 200,
            height: 150,
            order,
        }
    }

    fn favorite_value(gif: &FavoriteGif) -> Vec<u8> {
        let mut value = uint(1, u64::from(gif.format));
        value.extend(field(2, gif.src.as_bytes()));
        value.extend(uint(3, u64::from(gif.width)));
        value.extend(uint(4, u64::from(gif.height)));
        value.extend(uint(5, u64::from(gif.order)));
        value
    }

    fn favorite_map_entry(gif: &FavoriteGif) -> Vec<u8> {
        let mut entry = field(1, gif.url.as_bytes());
        entry.extend(field(2, &favorite_value(gif)));
        entry
    }

    fn frecency_fixture(gifs: &[FavoriteGif]) -> Vec<u8> {
        let mut favorites = uint(2, 1);
        for gif in gifs {
            favorites.extend(field(1, &favorite_map_entry(gif)));
        }
        favorites.extend(field(9, b"unknown-subfield"));
        let mut bytes = field(1, &uint(1, 3));
        bytes.extend(field(2, &favorites));
        bytes.extend(field(5, &field(1, &[])));
        bytes.extend(field(6, &[8, 5]));
        bytes.extend(field(99, b"unknown-top-level"));
        bytes
    }

    #[test]
    fn favorite_gifs_decode_and_sort_newest_first() {
        let older = favorite(
            "https://klipy.com/gifs/older",
            "https://static.klipy.com/older.webp",
            3,
        );
        let newer = favorite(
            "https://klipy.com/gifs/newer",
            "https://static.klipy.com/newer.webp",
            9,
        );
        let decoded = favorite_gifs(&frecency_fixture(&[newer.clone(), older.clone()])).unwrap();
        assert_eq!(decoded, [newer, older]);
        assert!(favorite_gifs(&[]).unwrap().is_empty());
        assert!(favorite_gifs(&frecency_fixture(&[])).unwrap().is_empty());
    }

    #[test]
    fn with_favorite_gifs_replaces_only_map_entries() {
        let old = favorite(
            "https://klipy.com/gifs/old",
            "https://static.klipy.com/old.webp",
            1,
        );
        let first = favorite(
            "https://klipy.com/gifs/first",
            "https://static.klipy.com/first.webp",
            2,
        );
        let second = favorite(
            "https://klipy.com/gifs/second",
            "https://static.klipy.com/second.webp",
            7,
        );
        let original = frecency_fixture(&[old]);

        let mut expected_favorites = uint(2, 1);
        expected_favorites.extend(field(9, b"unknown-subfield"));
        expected_favorites.extend(field(1, &favorite_map_entry(&first)));
        expected_favorites.extend(field(1, &favorite_map_entry(&second)));
        let mut expected = field(1, &uint(1, 3));
        expected.extend(field(2, &expected_favorites));
        expected.extend(field(5, &field(1, &[])));
        expected.extend(field(6, &[8, 5]));
        expected.extend(field(99, b"unknown-top-level"));

        let updated = with_favorite_gifs(&original, &[first.clone(), second.clone()]).unwrap();
        assert_eq!(updated, expected);
        assert_eq!(favorite_gifs(&updated).unwrap(), [second, first]);

        let mut expected_favorites = uint(2, 1);
        expected_favorites.extend(field(9, b"unknown-subfield"));
        let mut expected_empty = field(1, &uint(1, 3));
        expected_empty.extend(field(2, &expected_favorites));
        expected_empty.extend(field(5, &field(1, &[])));
        expected_empty.extend(field(6, &[8, 5]));
        expected_empty.extend(field(99, b"unknown-top-level"));

        let removed = with_favorite_gifs(&updated, &[]).unwrap();
        assert_eq!(removed, expected_empty);
        assert!(favorite_gifs(&removed).unwrap().is_empty());
    }

    #[test]
    fn with_favorite_gifs_creates_the_favorites_field_when_missing() {
        let gif = favorite(
            "https://klipy.com/gifs/new",
            "https://static.klipy.com/new.webp",
            1,
        );
        let bare = field(1, &uint(1, 3));
        let updated = with_favorite_gifs(&bare, std::slice::from_ref(&gif)).unwrap();
        assert_eq!(&updated[..bare.len()], &bare[..]);
        assert_eq!(favorite_gifs(&updated).unwrap(), [gif]);
    }

    #[test]
    fn merge_partial_replaces_only_the_fields_carried_by_the_update() {
        let base = [field(1, b"one"), field(2, b"two"), field(5, b"five")].concat();
        let update = [field(2, b"new-two"), field(6, b"six"), uint(7, 9)].concat();
        assert_eq!(
            merge_partial(&base, &update).unwrap(),
            [
                field(1, b"one"),
                field(2, b"new-two"),
                field(5, b"five"),
                field(6, b"six"),
                uint(7, 9)
            ]
            .concat()
        );
        let repeated = [field(1, b"one"), field(2, b"a"), field(2, b"b")].concat();
        let replacement = [field(2, b"c"), field(2, b"d")].concat();
        assert_eq!(
            merge_partial(&repeated, &replacement).unwrap(),
            [field(1, b"one"), field(2, b"c"), field(2, b"d")].concat()
        );
        assert_eq!(merge_partial(&[], &[]).unwrap(), Vec::<u8>::new());
        assert_eq!(merge_partial(&base, &[]).unwrap(), base);
    }

    #[test]
    fn malformed_frecency_input_is_rejected_without_panicking() {
        for bytes in [
            &[18][..],
            &[18, 5, 10][..],
            &[18, 3, 10, 1, b'x'][..],
            &[0xff; 11][..],
            &[0][..],
        ] {
            assert!(favorite_gifs(bytes).is_err());
        }
        for bytes in [&[18][..], &[18, 5, 10][..], &[0xff; 11][..], &[0][..]] {
            assert!(with_favorite_gifs(bytes, &[]).is_err());
        }
        let valid = frecency_fixture(&[]);
        for length in 0..valid.len() {
            let _ = favorite_gifs(&valid[..length]);
            let _ = with_favorite_gifs(&valid[..length], &[]);
            let _ = merge_partial(&valid[..length], &valid);
        }
        assert!(merge_partial(&valid, &[18, 9]).is_err());
    }
}
