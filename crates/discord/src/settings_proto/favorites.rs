use super::activity::{bool_field, rebuild};
use super::*;

const FAVORITES: u32 = 15;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Favorites {
    pub channels: Vec<FavoriteChannel>,
    pub muted: bool,
    pub guild_visible: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FavoriteChannel {
    pub id: u64,
    pub nickname: String,
    pub kind: u32,
    pub position: u32,
    pub parent_id: u64,
    pub channel_type: Option<u32>,
    pub collapsed: bool,
}

fn channel_value(bytes: &[u8], channel: &mut FavoriteChannel) -> Result<()> {
    let mut reader = Reader::new(bytes);
    while let Some((field, wire)) = reader.tag()? {
        match (field, wire) {
            (1, 2) => {
                channel.nickname = std::str::from_utf8(reader.bytes(wire)?)
                    .map_err(|_| DecodeError)?
                    .to_owned()
            }
            (2, 0) => channel.kind = u32::try_from(reader.varint()?).map_err(|_| DecodeError)?,
            (3, 0) => {
                channel.position = u32::try_from(reader.varint()?).map_err(|_| DecodeError)?
            }
            (4, 1) => channel.parent_id = reader.fixed64()?,
            (5, 2) => {
                channel.channel_type = Some(
                    u32::try_from(number_wrapper(reader.bytes(wire)?)?).map_err(|_| DecodeError)?,
                )
            }
            (6, 0) => channel.collapsed = reader.varint()? != 0,
            (1..=6, _) => return Err(DecodeError),
            _ => reader.skip(field, wire, 0)?,
        }
    }
    Ok(())
}

fn channel_entry(bytes: &[u8]) -> Result<FavoriteChannel> {
    let mut channel = FavoriteChannel::default();
    let mut reader = Reader::new(bytes);
    while let Some((field, wire)) = reader.tag()? {
        match (field, wire) {
            (1, 1) => channel.id = reader.fixed64()?,
            (2, 2) => channel_value(reader.bytes(wire)?, &mut channel)?,
            (1 | 2, _) => return Err(DecodeError),
            _ => reader.skip(field, wire, 0)?,
        }
    }
    Ok(channel)
}

pub(super) fn decode_message(bytes: &[u8], favorites: &mut Favorites) -> Result<()> {
    let mut reader = Reader::new(bytes);
    while let Some((field, wire)) = reader.tag()? {
        match (field, wire) {
            (1, 2) => {
                let channel = channel_entry(reader.bytes(wire)?)?;
                if let Some(existing) = favorites.channels.iter_mut().find(|c| c.id == channel.id) {
                    *existing = channel;
                } else {
                    favorites.channels.push(channel);
                }
            }
            (2, 0) => favorites.muted = reader.varint()? != 0,
            (3, 2) => favorites.guild_visible = number_wrapper(reader.bytes(wire)?)? != 0,
            (1..=3, _) => return Err(DecodeError),
            _ => reader.skip(field, wire, 0)?,
        }
    }
    Ok(())
}

pub fn favorites(preloaded: &[u8]) -> Result<Favorites> {
    let mut favorites = Favorites::default();
    for top in fields(preloaded)? {
        if top.number == FAVORITES {
            decode_message(top.value.ok_or(DecodeError)?, &mut favorites)?;
        }
    }
    Ok(favorites)
}

pub fn favorites_visible_patch(preloaded: &[u8], visible: bool) -> Result<Option<Vec<u8>>> {
    if favorites(preloaded)?.guild_visible == visible {
        return Ok(None);
    }
    let mut encoded = Vec::new();
    bool_field(&mut encoded, 3, visible);
    rebuild(preloaded, FAVORITES, &[3], &encoded).map(Some)
}

fn encode_entry(channel: &FavoriteChannel, existing: &[&[u8]]) -> Result<Vec<u8>> {
    let mut entry = Vec::new();
    let mut value = Vec::new();
    for bytes in existing {
        for field in fields(bytes)? {
            match field.number {
                1 => {}
                2 => {
                    for sub in fields(field.value.ok_or(DecodeError)?)? {
                        if !matches!(sub.number, 1..=6) {
                            value.extend_from_slice(sub.raw);
                        }
                    }
                }
                _ => entry.extend_from_slice(field.raw),
            }
        }
    }
    push_bytes(&mut value, 1, channel.nickname.as_bytes());
    push_uint(&mut value, 2, u64::from(channel.kind));
    push_uint(&mut value, 3, u64::from(channel.position));
    push_varint(&mut value, (4 << 3) | 1);
    value.extend(channel.parent_id.to_le_bytes());
    if let Some(channel_type) = channel.channel_type {
        let mut wrapper = Vec::new();
        push_uint(&mut wrapper, 1, u64::from(channel_type));
        push_bytes(&mut value, 5, &wrapper);
    }
    push_uint(&mut value, 6, u64::from(channel.collapsed));
    push_varint(&mut entry, (1 << 3) | 1);
    entry.extend(channel.id.to_le_bytes());
    push_bytes(&mut entry, 2, &value);
    let mut encoded = Vec::new();
    push_bytes(&mut encoded, 1, &entry);
    Ok(encoded)
}

pub fn favorite_channel_patch(
    preloaded: &[u8],
    channel_id: u64,
    desired: Option<FavoriteChannel>,
) -> Result<Option<Vec<u8>>> {
    if desired
        .as_ref()
        .is_some_and(|channel| channel.id != channel_id)
    {
        return Err(DecodeError);
    }
    let current = favorites(preloaded)?;
    if current.channels.iter().find(|c| c.id == channel_id) == desired.as_ref() {
        return Ok(None);
    }
    let mut siblings = Vec::new();
    let mut existing = Vec::new();
    for top in fields(preloaded)? {
        if top.number != FAVORITES {
            continue;
        }
        for field in fields(top.value.ok_or(DecodeError)?)? {
            let is_target = field.number == 1
                && channel_entry(field.value.ok_or(DecodeError)?)?.id == channel_id;
            if is_target {
                existing.push(field.value.ok_or(DecodeError)?);
            }
            siblings.push((field.raw, is_target));
        }
    }
    let encoded = desired
        .as_ref()
        .map(|channel| encode_entry(channel, &existing))
        .transpose()?;
    let mut content = Vec::new();
    let mut written = false;
    for (raw, is_target) in siblings {
        if is_target {
            if !written {
                content.extend_from_slice(encoded.as_deref().unwrap_or_default());
                written = true;
            }
        } else {
            content.extend_from_slice(raw);
        }
    }
    if !written {
        content.extend_from_slice(encoded.as_deref().unwrap_or_default());
    }
    let mut patch = Vec::new();
    push_bytes(&mut patch, FAVORITES, &content);
    Ok(Some(patch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_fixtures::{field, uint};

    fn channel(id: u64) -> FavoriteChannel {
        FavoriteChannel {
            id,
            nickname: "general".into(),
            kind: 1,
            position: 3,
            parent_id: 99,
            channel_type: Some(0),
            collapsed: true,
        }
    }

    #[test]
    fn defaults_and_channel_values() {
        assert_eq!(favorites(&[]).unwrap(), Favorites::default());
        assert_eq!(favorites(&field(15, &[])).unwrap(), Favorites::default());
        let favorite = channel(u64::MAX);
        let bytes = field(
            15,
            &[
                encode_entry(&favorite, &[]).unwrap(),
                uint(2, 1),
                field(3, &uint(1, 1)),
            ]
            .concat(),
        );
        assert_eq!(
            favorites(&bytes).unwrap(),
            Favorites {
                channels: vec![favorite],
                muted: true,
                guild_visible: true
            }
        );
        assert_eq!(
            favorites(&field(15, &field(1, &[]))).unwrap().channels,
            [FavoriteChannel::default()]
        );
        let mut category = channel(99);
        category.kind = 2;
        category.channel_type = None;
        let bytes = field(15, &encode_entry(&category, &[]).unwrap());
        assert_eq!(favorites(&bytes).unwrap().channels, [category]);
    }

    #[test]
    fn visibility_patch_preserves_entries_and_opaque_fields() {
        let entry = encode_entry(&channel(123), &[]).unwrap();
        let opaque = field(7, b"opaque");
        let base = field(15, &[entry.clone(), uint(2, 1), opaque.clone()].concat());
        assert_eq!(favorites_visible_patch(&base, false).unwrap(), None);
        let patch = favorites_visible_patch(&base, true).unwrap().unwrap();
        assert_eq!(
            patch,
            field(
                15,
                &[entry, uint(2, 1), opaque, field(3, &uint(1, 1))].concat()
            )
        );
        assert!(favorites(&patch).unwrap().guild_visible);
        let off = favorites_visible_patch(&patch, false).unwrap().unwrap();
        assert!(!favorites(&off).unwrap().guild_visible);
    }

    #[test]
    fn insert_replace_remove_preserve_other_entries_and_opaque_fields() {
        let first = channel(123);
        let second = channel(456);
        let entry_opaque = field(99, b"entry");
        let value_opaque = field(88, b"value");
        let old = [
            vec![9],
            first.id.to_le_bytes().to_vec(),
            field(2, &value_opaque),
            entry_opaque.clone(),
        ]
        .concat();
        let mut base = field(
            15,
            &[
                encode_entry(&first, &[&old]).unwrap(),
                encode_entry(&second, &[]).unwrap(),
                field(7, b"future"),
                uint(2, 1),
            ]
            .concat(),
        );
        let edited = FavoriteChannel {
            nickname: "new".into(),
            ..first.clone()
        };
        let patch = favorite_channel_patch(&base, first.id, Some(edited.clone()))
            .unwrap()
            .unwrap();
        let top = fields(&patch).unwrap();
        let content = fields(top[0].value.unwrap()).unwrap();
        assert!(
            content
                .iter()
                .any(|f| f.raw == encode_entry(&second, &[]).unwrap())
        );
        assert!(content.iter().any(|f| f.raw == field(7, b"future")));
        let target = content
            .iter()
            .find(|f| f.number == 1 && channel_entry(f.value.unwrap()).unwrap().id == first.id)
            .unwrap();
        let entry = fields(target.value.unwrap()).unwrap();
        assert!(entry.iter().any(|f| f.raw == entry_opaque));
        let value = fields(entry.iter().find(|f| f.number == 2).unwrap().value.unwrap()).unwrap();
        assert!(value.iter().any(|f| f.raw == value_opaque));
        assert_eq!(
            favorites(&patch).unwrap().channels,
            [edited.clone(), second.clone()]
        );
        base = merge_partial(&base, &patch).unwrap();
        assert_eq!(
            favorite_channel_patch(&base, first.id, Some(edited)).unwrap(),
            None
        );
        let third = channel(789);
        let patch = favorite_channel_patch(&base, third.id, Some(third.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(favorites(&patch).unwrap().channels.last(), Some(&third));
        let patch = favorite_channel_patch(&base, first.id, None)
            .unwrap()
            .unwrap();
        assert_eq!(favorites(&patch).unwrap().channels, [second]);
        assert!(favorites(&patch).unwrap().muted);
        assert_eq!(favorite_channel_patch(&base, 999, None).unwrap(), None);
    }

    #[test]
    fn duplicate_map_keys_use_last_value_and_remove_all_copies() {
        let first = channel(123);
        let last = FavoriteChannel {
            nickname: "last".into(),
            ..first.clone()
        };
        let base = field(
            15,
            &[
                encode_entry(&first, &[]).unwrap(),
                encode_entry(&last, &[]).unwrap(),
            ]
            .concat(),
        );
        assert_eq!(favorites(&base).unwrap().channels, [last]);
        assert!(
            favorites(&favorite_channel_patch(&base, 123, None).unwrap().unwrap())
                .unwrap()
                .channels
                .is_empty()
        );
    }

    #[test]
    fn malformed_entries_and_mismatched_ids_fail_closed() {
        for bytes in [
            field(15, &field(1, &uint(1, 7))),
            field(15, &field(1, &field(2, &uint(4, 1)))),
            field(15, &uint(3, 1)),
        ] {
            assert!(favorites(&bytes).is_err());
            assert!(favorites_visible_patch(&bytes, true).is_err());
            assert!(favorite_channel_patch(&bytes, 7, None).is_err());
        }
        assert!(favorite_channel_patch(&[], 7, Some(channel(8))).is_err());
    }
}
