use super::model::{Channel, Guild, Member, Overwrite};

const VIEW_CHANNEL: u64 = 1 << 10;
const ADMINISTRATOR: u64 = 1 << 3;

fn apply_overwrites(
    mut permissions: u64,
    overwrites: &[Overwrite],
    guild_id: u64,
    user_id: u64,
    roles: &[u64],
) -> u64 {
    if let Some(everyone) = overwrites.iter().find(|o| o.kind == 0 && o.id == guild_id) {
        permissions = (permissions & !everyone.deny) | everyone.allow;
    }
    let (mut allow, mut deny) = (0, 0);
    for overwrite in overwrites
        .iter()
        .filter(|o| o.kind == 0 && o.id != guild_id && roles.contains(&o.id))
    {
        allow |= overwrite.allow;
        deny |= overwrite.deny;
    }
    permissions = (permissions & !deny) | allow;
    if let Some(member) = overwrites.iter().find(|o| o.kind == 1 && o.id == user_id) {
        permissions = (permissions & !member.deny) | member.allow;
    }
    permissions
}

/// Channel overwrite sets are authoritative, not stacked on category overwrites. Synced
/// children carry the same set; only an omitted set falls back to its category's set.
/// https://docs.discord.food/topics/permissions#permission-syncing
pub(crate) fn can_view(guild: &Guild, channel: &Channel, user_id: u64) -> bool {
    if guild.owner_id() == Some(user_id) {
        return true;
    }
    let member = guild
        .members
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|m| m.id() == Some(user_id));
    let Some(Member {
        roles: member_roles,
        ..
    }) = member
    else {
        return false;
    };
    let roles = guild.roles.as_deref().unwrap_or_default();
    let mut permissions = roles
        .iter()
        .filter(|r| r.id == guild.id || member_roles.contains(&r.id))
        .fold(0, |p, role| p | role.permissions);
    if permissions & ADMINISTRATOR != 0 {
        return true;
    }
    let parent = channel
        .parent_id
        .flatten()
        .and_then(|id| guild.channels.as_deref()?.iter().find(|c| c.id == id));
    let overwrites = channel
        .permission_overwrites
        .as_deref()
        .or_else(|| parent.and_then(|c| c.permission_overwrites.as_deref()))
        .unwrap_or_default();
    permissions = apply_overwrites(permissions, overwrites, guild.id, user_id, member_roles);
    permissions & VIEW_CHANNEL != 0
}
