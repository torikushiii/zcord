//! Schemas: https://github.com/discord-userdoccers/discord-protos/tree/master/discord_protos/discord_users/v1
use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::{HashMap, HashSet};

mod activity;
pub use activity::{ActivityPrivacy, NewServerSharing, activity_privacy, activity_privacy_patch};
mod appearance;
pub use appearance::{ClientTheme, CustomTheme, appearance_patch, client_theme};
mod favorites;
pub use favorites::{
    FavoriteChannel, Favorites, favorite_channel_patch, favorites, favorites_visible_patch,
};
mod text_images;
pub use text_images::{SpoilerMode, StickerAnimation, TextImages, text_images, text_images_patch};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuildFolder {
    pub guild_ids: Vec<u64>,
    pub id: Option<u64>,
    pub name: Option<String>,
    pub color: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountTheme {
    Dark,
    Light,
    Darker,
    Midnight,
}

impl AccountTheme {
    pub fn wire(self) -> u64 {
        match self {
            Self::Dark => 1,
            Self::Light => 2,
            Self::Darker => 3,
            Self::Midnight => 4,
        }
    }

    pub fn from_wire(value: u64) -> Option<Self> {
        match value {
            1 => Some(Self::Dark),
            2 => Some(Self::Light),
            3 => Some(Self::Darker),
            4 => Some(Self::Midnight),
            _ => None,
        }
    }
}

#[derive(Default, Debug, PartialEq)]
pub struct Settings {
    pub folders: Option<Vec<GuildFolder>>,
    pub status: Option<StatusSettings>,
    pub theme: Option<AccountTheme>,
    pub client_theme: Option<ClientTheme>,
    pub appearance_present: bool,
    pub text_images: Option<TextImages>,
    pub favorites: Option<Favorites>,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct StatusSettings {
    pub status: Option<String>,
    pub custom_status: Option<CustomStatus>,
    pub custom_status_present: bool,
    pub show_current_game: Option<bool>,
}

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
            3 => {
                let mut bool_reader = Reader::new(reader.bytes(wire)?);
                let mut value = false;
                while let Some((field, wire)) = bool_reader.tag()? {
                    if field == 1 {
                        if wire != 0 {
                            return Err(DecodeError);
                        }
                        value = bool_reader.varint()? != 0;
                    } else {
                        bool_reader.skip(field, wire, 0)?;
                    }
                }
                status.show_current_game = Some(value);
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
                    2 => fixed_ids(&mut guilds, wire, &mut Vec::new())?,
                    _ => guilds.skip(field, wire, 0)?,
                }
            }
        } else if field == 11 {
            settings.status = Some(status_settings(reader.bytes(wire)?)?);
        } else if field == 13 {
            (settings.theme, settings.client_theme) =
                appearance::decode_message(reader.bytes(wire)?)?;
            settings.appearance_present = true;
        } else if field == 6 {
            text_images::decode_message(
                reader.bytes(wire)?,
                settings.text_images.get_or_insert_with(TextImages::default),
            )?;
        } else if field == 15 {
            favorites::decode_message(
                reader.bytes(wire)?,
                settings.favorites.get_or_insert_with(Favorites::default),
            )?;
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

pub fn status_patch(preloaded: &[u8], status: &str) -> Result<Vec<u8>> {
    status_patch_at(preloaded, status, 0, None)
}

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

/// Status settings are replaced whole on PATCH, so every other status field is copied from
/// `preloaded`; leaving `custom` out clears the custom status.
/// https://docs.discord.food/resources/user-settings-proto
pub fn custom_status_patch(
    preloaded: &[u8],
    custom: Option<&CustomStatus>,
    created_at_ms: u64,
) -> Result<Vec<u8>> {
    let mut content = Vec::new();
    for field in fields(preloaded)? {
        if field.number != 11 {
            continue;
        }
        for sub in fields(field.value.ok_or(DecodeError)?)? {
            if sub.number != 2 {
                content.extend_from_slice(sub.raw);
            }
        }
    }
    if let Some(custom) = custom {
        let mut status = Vec::new();
        if !custom.text.is_empty() {
            push_bytes(&mut status, 1, custom.text.as_bytes());
        }
        if custom.emoji_id != 0 {
            push_fixed64(&mut status, 2, custom.emoji_id);
        }
        if !custom.emoji_name.is_empty() {
            push_bytes(&mut status, 3, custom.emoji_name.as_bytes());
        }
        if custom.expires_at_ms != 0 {
            push_fixed64(&mut status, 4, custom.expires_at_ms);
        }
        push_fixed64(&mut status, 5, created_at_ms);
        push_bytes(&mut content, 2, &status);
    }
    let mut patch = Vec::new();
    push_bytes(&mut patch, 11, &content);
    Ok(patch)
}

pub fn theme_patch(preloaded: &[u8], theme: AccountTheme) -> Result<Vec<u8>> {
    let mut content = Vec::new();
    for field in fields(preloaded)? {
        if field.number != 13 {
            continue;
        }
        for sub in fields(field.value.ok_or(DecodeError)?)? {
            if sub.number != 1 {
                content.extend_from_slice(sub.raw);
            }
        }
    }
    push_uint(&mut content, 1, theme.wire());
    let mut patch = Vec::new();
    push_bytes(&mut patch, 13, &content);
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

fn push_fixed64(out: &mut Vec<u8>, field: u32, value: u64) {
    push_varint(out, (u64::from(field) << 3) | 1);
    out.extend_from_slice(&value.to_le_bytes());
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
pub(crate) mod tests;
