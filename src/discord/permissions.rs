use super::model::{Channel, Guild, Member, Overwrite};

pub const VIEW_CHANNEL: u64 = 1 << 10;
pub const SEND_MESSAGES: u64 = 1 << 11;
pub const MANAGE_MESSAGES: u64 = 1 << 13;
pub const USE_EXTERNAL_EMOJIS: u64 = 1 << 18;
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
pub fn channel_permissions(guild: &Guild, channel: &Channel, user_id: u64) -> u64 {
    if guild.owner_id() == Some(user_id) {
        return u64::MAX;
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
        return 0;
    };
    let roles = guild.roles.as_deref().unwrap_or_default();
    let permissions = roles
        .iter()
        .filter(|r| r.id == guild.id || member_roles.contains(&r.id))
        .fold(0, |p, role| p | role.permissions);
    if permissions & ADMINISTRATOR != 0 {
        return u64::MAX;
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
    apply_overwrites(permissions, overwrites, guild.id, user_id, member_roles)
}

pub fn can_view(guild: &Guild, channel: &Channel, user_id: u64) -> bool {
    channel_permissions(guild, channel, user_id) & VIEW_CHANNEL != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::model;
    use serde_json::json;

    fn ready() -> model::Ready {
        serde_json::from_str(include_str!("fixtures/ready.json")).unwrap()
    }

    #[test]
    fn send_manage_and_role_grants() {
        let mut ready = ready();
        let guild = &mut ready.guilds[0];
        let plain: model::Channel = serde_json::from_value(json!({"id":"3001","type":0})).unwrap();
        assert_eq!(
            channel_permissions(guild, &plain, 1) & VIEW_CHANNEL,
            VIEW_CHANNEL
        );
        assert_eq!(channel_permissions(guild, &plain, 1) & SEND_MESSAGES, 0);
        assert_eq!(channel_permissions(guild, &plain, 1) & MANAGE_MESSAGES, 0);
        let denied: model::Channel = serde_json::from_value(json!({"id":"3002","type":0,
            "permission_overwrites":[{"id":"1","type":1,"allow":"0","deny":"2048"}]}))
        .unwrap();
        assert_eq!(channel_permissions(guild, &denied, 1) & SEND_MESSAGES, 0);
        assert_eq!(
            channel_permissions(guild, &denied, 1) & VIEW_CHANNEL,
            VIEW_CHANNEL
        );
        guild.roles.as_mut().unwrap()[1].permissions = MANAGE_MESSAGES;
        assert_eq!(
            channel_permissions(guild, &plain, 1) & MANAGE_MESSAGES,
            MANAGE_MESSAGES
        );
        assert_eq!(
            channel_permissions(guild, &denied, 1) & MANAGE_MESSAGES,
            MANAGE_MESSAGES
        );
        guild.roles.as_mut().unwrap()[1].permissions = ADMINISTRATOR;
        assert_eq!(channel_permissions(guild, &denied, 1), u64::MAX);
        guild.roles.as_mut().unwrap()[1].permissions = 0;
        let owned = ready.guilds.iter().find(|guild| guild.id == 100).unwrap();
        let hidden = owned
            .channels
            .as_ref()
            .unwrap()
            .iter()
            .find(|channel| channel.id == 1001)
            .unwrap();
        assert_eq!(channel_permissions(owned, hidden, 1), u64::MAX);
    }

    #[test]
    fn permission_admin_member_and_category_rules() {
        let mut ready = ready();
        let guild = &mut ready.guilds[0];
        let member_deny: model::Channel =
            serde_json::from_value(json!({"id":"2010", "type":0, "permission_overwrites":[
            {"id":"201", "type":0, "allow":"1024", "deny":"0"},
            {"id":"1", "type":1, "allow":"0", "deny":"1024"}]}))
            .unwrap();
        assert!(!can_view(guild, &member_deny, 1));
        guild.roles.as_mut().unwrap()[1].permissions = 8;
        assert!(can_view(guild, &member_deny, 1));
        guild.roles.as_mut().unwrap()[1].permissions = 0;
        let parent: model::Channel = serde_json::from_value(json!({"id":"230", "type":4, "permission_overwrites":[{"id":"200", "type":0, "deny":"1024", "allow":"0"}]})).unwrap();
        guild.channels.as_mut().unwrap().push(parent);
        let mut child: model::Channel =
            serde_json::from_value(json!({"id":"2011", "type":0, "parent_id":"230"})).unwrap();
        assert!(!can_view(guild, &child, 1));
        child.permission_overwrites = Some(Vec::new());
        assert!(can_view(guild, &child, 1));
    }
}
