//! Unset wrappers read as on, the server's default.
use super::*;

const NOTIFICATIONS: u32 = 7;
const PRIVACY: u32 = 8;
const STATUS: u32 = 11;

const NOTIFY_COME_ONLINE: u32 = 25;
const NOTIFY_GO_LIVE: u32 = 26;
const JOIN_FRIENDS: u32 = 1;
const JOIN_VOICE: u32 = 2;
const ACTIVITY_RESTRICTED_GUILDS: u32 = 13;
const NEW_GUILDS_ACTIVITY: u32 = 29;
const SHOW_CURRENT_GAME: u32 = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NewServerSharing {
    #[default]
    All,
    SmallOnly,
    Nowhere,
}

impl NewServerSharing {
    fn from_wire(value: u64) -> Self {
        match value {
            2 => Self::SmallOnly,
            3 => Self::Nowhere,
            _ => Self::All,
        }
    }

    fn wire(self) -> u64 {
        match self {
            Self::All => 1,
            Self::SmallOnly => 2,
            Self::Nowhere => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityPrivacy {
    pub share_activity: bool,
    pub share_come_online: bool,
    pub share_go_live: bool,
    pub new_servers: NewServerSharing,
    pub hidden_guild_ids: Vec<u64>,
    pub join_friends: bool,
    pub join_voice: bool,
}

impl Default for ActivityPrivacy {
    fn default() -> Self {
        Self {
            share_activity: true,
            share_come_online: true,
            share_go_live: true,
            new_servers: NewServerSharing::All,
            hidden_guild_ids: Vec::new(),
            join_friends: true,
            join_voice: true,
        }
    }
}

impl ActivityPrivacy {
    pub fn rebase(&self, base: &Self, onto: &Self) -> Self {
        fn pick<T: PartialEq + Clone>(edited: &T, base: &T, onto: &T) -> T {
            if edited == base { onto } else { edited }.clone()
        }
        let mut hidden_guild_ids: Vec<u64> = onto
            .hidden_guild_ids
            .iter()
            .copied()
            .filter(|id| {
                !(base.hidden_guild_ids.contains(id) && !self.hidden_guild_ids.contains(id))
            })
            .collect();
        for id in &self.hidden_guild_ids {
            if !base.hidden_guild_ids.contains(id) && !hidden_guild_ids.contains(id) {
                hidden_guild_ids.push(*id);
            }
        }
        Self {
            share_activity: pick(
                &self.share_activity,
                &base.share_activity,
                &onto.share_activity,
            ),
            share_come_online: pick(
                &self.share_come_online,
                &base.share_come_online,
                &onto.share_come_online,
            ),
            share_go_live: pick(
                &self.share_go_live,
                &base.share_go_live,
                &onto.share_go_live,
            ),
            new_servers: pick(&self.new_servers, &base.new_servers, &onto.new_servers),
            hidden_guild_ids,
            join_friends: pick(&self.join_friends, &base.join_friends, &onto.join_friends),
            join_voice: pick(&self.join_voice, &base.join_voice, &onto.join_voice),
        }
    }
}

fn bool_value(field: &Field<'_>) -> Result<bool> {
    let mut reader = Reader::new(field.value.ok_or(DecodeError)?);
    let mut value = false;
    while let Some((number, wire)) = reader.tag()? {
        if number == 1 && wire == 0 {
            value = reader.varint()? != 0;
        } else {
            reader.skip(number, wire, 0)?;
        }
    }
    Ok(value)
}

fn varint_value(field: &Field<'_>) -> Result<u64> {
    let mut reader = Reader::new(field.raw);
    reader.tag()?.ok_or(DecodeError)?;
    reader.varint()
}

pub fn activity_privacy(preloaded: &[u8]) -> Result<ActivityPrivacy> {
    let mut privacy = ActivityPrivacy::default();
    for top in fields(preloaded)? {
        if !matches!(top.number, NOTIFICATIONS | PRIVACY | STATUS) {
            continue;
        }
        for field in fields(top.value.ok_or(DecodeError)?)? {
            match (top.number, field.number) {
                (NOTIFICATIONS, NOTIFY_COME_ONLINE) => {
                    privacy.share_come_online = bool_value(&field)?;
                }
                (NOTIFICATIONS, NOTIFY_GO_LIVE) => privacy.share_go_live = bool_value(&field)?,
                (PRIVACY, JOIN_FRIENDS) => privacy.join_friends = bool_value(&field)?,
                (PRIVACY, JOIN_VOICE) => privacy.join_voice = bool_value(&field)?,
                (PRIVACY, ACTIVITY_RESTRICTED_GUILDS) => {
                    let mut reader = Reader::new(field.raw);
                    reader.tag()?;
                    fixed_ids(&mut reader, field.wire, &mut privacy.hidden_guild_ids)?;
                }
                (PRIVACY, NEW_GUILDS_ACTIVITY) if field.wire == 0 => {
                    privacy.new_servers = NewServerSharing::from_wire(varint_value(&field)?);
                }
                (STATUS, SHOW_CURRENT_GAME) => privacy.share_activity = bool_value(&field)?,
                _ => {}
            }
        }
    }
    Ok(privacy)
}

pub(super) fn bool_field(out: &mut Vec<u8>, number: u32, value: bool) {
    let mut wrapper = Vec::new();
    if value {
        push_uint(&mut wrapper, 1, 1);
    }
    push_bytes(out, number, &wrapper);
}

/// PATCH replaces whole top-level messages, so every field not `replaced` is copied over.
pub(super) fn rebuild(
    preloaded: &[u8],
    top: u32,
    replaced: &[u32],
    encoded: &[u8],
) -> Result<Vec<u8>> {
    let mut content = Vec::new();
    for field in fields(preloaded)? {
        if field.number != top {
            continue;
        }
        for sub in fields(field.value.ok_or(DecodeError)?)? {
            if !replaced.contains(&sub.number) {
                content.extend_from_slice(sub.raw);
            }
        }
    }
    content.extend_from_slice(encoded);
    let mut message = Vec::new();
    push_bytes(&mut message, top, &content);
    Ok(message)
}

pub fn activity_privacy_patch(
    preloaded: &[u8],
    desired: &ActivityPrivacy,
) -> Result<Option<Vec<u8>>> {
    let current = activity_privacy(preloaded)?;
    let mut patch = Vec::new();

    let mut replaced = Vec::new();
    let mut encoded = Vec::new();
    if current.share_come_online != desired.share_come_online {
        replaced.push(NOTIFY_COME_ONLINE);
        bool_field(&mut encoded, NOTIFY_COME_ONLINE, desired.share_come_online);
    }
    if current.share_go_live != desired.share_go_live {
        replaced.push(NOTIFY_GO_LIVE);
        bool_field(&mut encoded, NOTIFY_GO_LIVE, desired.share_go_live);
    }
    if !replaced.is_empty() {
        patch.extend(rebuild(preloaded, NOTIFICATIONS, &replaced, &encoded)?);
    }

    let mut replaced = Vec::new();
    let mut encoded = Vec::new();
    if current.join_friends != desired.join_friends {
        replaced.push(JOIN_FRIENDS);
        bool_field(&mut encoded, JOIN_FRIENDS, desired.join_friends);
    }
    if current.join_voice != desired.join_voice {
        replaced.push(JOIN_VOICE);
        bool_field(&mut encoded, JOIN_VOICE, desired.join_voice);
    }
    if current.hidden_guild_ids != desired.hidden_guild_ids {
        replaced.push(ACTIVITY_RESTRICTED_GUILDS);
        if !desired.hidden_guild_ids.is_empty() {
            let packed: Vec<u8> = desired
                .hidden_guild_ids
                .iter()
                .flat_map(|id| id.to_le_bytes())
                .collect();
            push_bytes(&mut encoded, ACTIVITY_RESTRICTED_GUILDS, &packed);
        }
    }
    if current.new_servers != desired.new_servers {
        replaced.push(NEW_GUILDS_ACTIVITY);
        push_uint(
            &mut encoded,
            NEW_GUILDS_ACTIVITY,
            desired.new_servers.wire(),
        );
    }
    if !replaced.is_empty() {
        patch.extend(rebuild(preloaded, PRIVACY, &replaced, &encoded)?);
    }

    if current.share_activity != desired.share_activity {
        let mut encoded = Vec::new();
        bool_field(&mut encoded, SHOW_CURRENT_GAME, desired.share_activity);
        patch.extend(rebuild(preloaded, STATUS, &[SHOW_CURRENT_GAME], &encoded)?);
    }
    Ok((!patch.is_empty()).then_some(patch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_fixtures::{field, uint};

    fn sample() -> Vec<u8> {
        let mut privacy = field(3, &7u64.to_le_bytes());
        privacy.extend(field(
            13,
            &[5u64.to_le_bytes(), 6u64.to_le_bytes()].concat(),
        ));
        privacy.extend(field(JOIN_VOICE, &[]));
        let mut status = field(1, &field(1, b"idle"));
        status.extend(field(3, &uint(1, 1)));
        [
            field(1, &uint(3, 9)),
            field(7, &field(NOTIFY_COME_ONLINE, &[])),
            field(8, &privacy),
            field(11, &status),
        ]
        .concat()
    }

    #[test]
    fn rebasing_keeps_changes_made_elsewhere() {
        let base = ActivityPrivacy {
            hidden_guild_ids: vec![1, 2],
            ..ActivityPrivacy::default()
        };
        let edited = ActivityPrivacy {
            join_voice: false,
            hidden_guild_ids: vec![2, 3],
            ..base.clone()
        };
        let elsewhere = ActivityPrivacy {
            share_activity: false,
            hidden_guild_ids: vec![1, 2, 4],
            ..base.clone()
        };
        let rebased = edited.rebase(&base, &elsewhere);
        assert!(
            !rebased.share_activity,
            "sharing stays as the other client set it"
        );
        assert!(!rebased.join_voice);
        assert_eq!(rebased.hidden_guild_ids, [2, 4, 3]);
    }

    #[test]
    fn unset_fields_read_as_discord_defaults() {
        assert_eq!(activity_privacy(&[]).unwrap(), ActivityPrivacy::default());
        let read = activity_privacy(&sample()).unwrap();
        assert!(!read.share_come_online);
        assert!(read.share_go_live);
        assert!(read.join_friends);
        assert!(!read.join_voice);
        assert_eq!(read.hidden_guild_ids, [5, 6]);
        assert_eq!(read.new_servers, NewServerSharing::All);
        assert!(read.share_activity);
    }

    #[test]
    fn a_patch_carries_only_changed_messages_and_keeps_their_other_fields() {
        let base = sample();
        let current = activity_privacy(&base).unwrap();
        assert_eq!(activity_privacy_patch(&base, &current).unwrap(), None);

        let desired = ActivityPrivacy {
            hidden_guild_ids: vec![6],
            new_servers: NewServerSharing::Nowhere,
            join_voice: true,
            ..current.clone()
        };
        let patch = activity_privacy_patch(&base, &desired).unwrap().unwrap();
        let tops: Vec<u32> = fields(&patch).unwrap().iter().map(|f| f.number).collect();
        assert_eq!(tops, [PRIVACY]);
        let merged = merge_partial(&base, &patch).unwrap();
        assert_eq!(activity_privacy(&merged).unwrap(), desired);
        let privacy = fields(&merged)
            .unwrap()
            .into_iter()
            .find(|f| f.number == PRIVACY)
            .unwrap();
        let kept = fields(privacy.value.unwrap()).unwrap();
        assert!(
            kept.iter().any(|f| f.number == 3),
            "unrelated fields survive"
        );

        let desired = ActivityPrivacy {
            share_activity: false,
            share_come_online: true,
            ..activity_privacy(&merged).unwrap()
        };
        let patch = activity_privacy_patch(&merged, &desired).unwrap().unwrap();
        let tops: Vec<u32> = fields(&patch).unwrap().iter().map(|f| f.number).collect();
        assert_eq!(tops, [NOTIFICATIONS, STATUS]);
        let merged = merge_partial(&merged, &patch).unwrap();
        assert_eq!(activity_privacy(&merged).unwrap(), desired);
        let status = decode(&merged).unwrap().status.unwrap();
        assert_eq!(status.status.as_deref(), Some("idle"));
    }
}
