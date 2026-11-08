use super::*;

impl Store {
    fn member_row(&mut self, guild_id: u64, item: &model::MemberItem) -> Option<MemberRow> {
        if let Some(group) = &item.group {
            let name = self
                .social
                .guilds
                .get(&guild_id)
                .map_or("Role", |guild| group_name(&guild.roles, &group.id));
            return Some(MemberRow::Group {
                name: name.to_owned(),
                count: group.count,
            });
        }
        let member = item.member.as_ref()?;
        let id = member.id()?;
        self.upsert_member(guild_id, id, member);
        Some(MemberRow::Member { user_id: id })
    }
    pub(super) fn apply_member_list(&mut self, update: &model::MemberListUpdate) -> bool {
        let guild = self.social.guilds.entry(update.guild_id).or_default();
        let switching = guild.list.as_ref().is_none_or(|list| list.id != update.id);
        if switching && !update.ops.iter().any(|op| op.op == "SYNC") {
            return false;
        }
        let mut groups = if switching {
            Vec::new()
        } else {
            std::mem::take(&mut guild.list_groups)
        };
        let mut list = if switching {
            MemberList {
                id: update.id.clone(),
                ..Default::default()
            }
        } else {
            guild.list.take().unwrap()
        };
        list.member_count = update.member_count;
        list.online_count = update.online_count;
        let total = update
            .groups
            .iter()
            .filter(|g| g.count > 0)
            .fold(0usize, |total, g| {
                total.saturating_add(g.count as usize + 1)
            });
        let max_rows = list
            .rows
            .len()
            .max(total)
            .saturating_add(update.ops.iter().filter(|op| op.op == "INSERT").count());
        for op in &update.ops {
            match op.op.as_str() {
                "SYNC" | "INVALIDATE" => {
                    let Some([start, end]) = op.range else {
                        continue;
                    };
                    if start > end || start as usize >= max_rows {
                        continue;
                    }
                    let end = (end as usize).min(max_rows.saturating_sub(1));
                    list.rows.resize(list.rows.len().max(end + 1), None);
                    groups.resize(list.rows.len(), None);
                    for (index, group) in groups
                        .iter_mut()
                        .enumerate()
                        .take(end + 1)
                        .skip(start as usize)
                    {
                        *group = if op.op == "SYNC" {
                            op.items
                                .get(index - start as usize)
                                .and_then(|item| item.group.as_ref())
                                .map(|g| g.id.clone())
                        } else {
                            None
                        };
                        list.rows[index] = if op.op == "SYNC" {
                            op.items
                                .get(index - start as usize)
                                .and_then(|item| self.member_row(update.guild_id, item))
                        } else {
                            None
                        };
                    }
                }
                "INSERT" | "UPDATE" => {
                    let Some(index) = op.index.map(|i| i as usize).filter(|i| *i < max_rows) else {
                        continue;
                    };
                    let row = op
                        .item
                        .as_ref()
                        .and_then(|item| self.member_row(update.guild_id, item));
                    if op.op == "INSERT" {
                        list.rows.resize(list.rows.len().max(index), None);
                        groups.resize(list.rows.len(), None);
                        list.rows.insert(index, row);
                        groups.insert(
                            index,
                            op.item
                                .as_ref()
                                .and_then(|item| item.group.as_ref())
                                .map(|g| g.id.clone()),
                        );
                    } else {
                        list.rows.resize(list.rows.len().max(index + 1), None);
                        groups.resize(list.rows.len(), None);
                        list.rows[index] = row;
                        groups[index] = op
                            .item
                            .as_ref()
                            .and_then(|item| item.group.as_ref())
                            .map(|g| g.id.clone());
                    }
                }
                "DELETE" => {
                    if let Some(index) = op
                        .index
                        .map(|i| i as usize)
                        .filter(|i| *i < list.rows.len())
                    {
                        list.rows.remove(index);
                        if index < groups.len() {
                            groups.remove(index);
                        }
                    }
                }
                _ => {}
            }
        }
        list.rows.resize(total, None);
        groups.resize(total, None);
        for (row, group) in list.rows.iter_mut().zip(&groups) {
            if let (Some(MemberRow::Group { count, .. }), Some(group)) = (row, group)
                && let Some(current) = update.groups.iter().find(|g| &g.id == group)
            {
                *count = current.count;
            }
        }
        let guild = self.social.guilds.entry(update.guild_id).or_default();
        guild.list = Some(list);
        guild.list_groups = groups;
        true
    }
}
