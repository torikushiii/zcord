use super::*;

impl Store {
    pub(super) fn rebuild_rail(&mut self) {
        let mut listed = std::collections::HashSet::new();
        let mut ordered = Vec::new();
        for folder in self.folder_layout.iter().flatten() {
            let ids: Vec<_> = folder
                .guild_ids
                .iter()
                .copied()
                .filter(|id| self.guild_indices.contains_key(id) && listed.insert(*id))
                .collect();
            if ids.is_empty() {
                continue;
            }
            if let Some(id) = folder.id {
                ordered.push(RailEntry::Folder(Folder {
                    id,
                    name: folder.name.clone(),
                    color: folder.color,
                    guild_ids: ids,
                }));
            } else {
                ordered.extend(ids.into_iter().map(RailEntry::Guild));
            }
        }
        self.rail = self
            .rail_order
            .iter()
            .copied()
            .filter(|id| self.guild_indices.contains_key(id) && !listed.contains(id))
            .map(RailEntry::Guild)
            .chain(ordered)
            .collect();
    }

    pub(super) fn guild_upsert(&mut self, update: Box<model::Guild>) -> Change {
        let id = update.id;
        if !self.guild_indices.contains_key(&id) {
            self.rail_order.insert(0, id);
            for folder in self.folder_layout.iter_mut().flatten() {
                folder.guild_ids.retain(|guild| *guild != id);
            }
        }
        if let Some(source) = &mut self.source {
            if let Some(guild) = source.guilds.iter_mut().find(|g| g.id == id) {
                guild.merge(&update);
            } else {
                source.guilds.push(*update);
            }
        }
        self.sync_guild_social(id);
        self.rebuild_structure(Some(id));
        Change::Structure
    }

    pub(super) fn guild_emojis_update(&mut self, update: model::GuildEmojisUpdate) -> Change {
        if let Some(guild) = self
            .source
            .as_mut()
            .and_then(|source| source.guilds.iter_mut().find(|g| g.id == update.guild_id))
        {
            guild.emojis = Some(update.emojis);
            let id = update.guild_id;
            self.rebuild_structure(Some(id));
            Change::Structure
        } else {
            Change::None
        }
    }

    pub(super) fn guild_stickers_update(&mut self, update: model::GuildStickersUpdate) -> Change {
        let Some(stickers) = update.stickers else {
            return Change::None;
        };
        let Some(guild) = self
            .source
            .as_mut()
            .and_then(|source| source.guilds.iter_mut().find(|g| g.id == update.guild_id))
        else {
            return Change::None;
        };
        guild.stickers = Some(stickers);
        Change::Structure
    }

    fn source_guild_mut(&mut self, id: u64) -> Option<&mut model::Guild> {
        self.source
            .as_mut()?
            .guilds
            .iter_mut()
            .find(|guild| guild.id == id)
    }

    pub(super) fn scheduled_event_upsert(&mut self, event: model::ScheduledEvent) -> Change {
        let Some(guild_id) = event.guild_id else {
            return Change::None;
        };
        let Some(guild) = self.source_guild_mut(guild_id) else {
            return Change::None;
        };
        let events = guild.guild_scheduled_events.get_or_insert_with(Vec::new);
        match events.iter_mut().find(|existing| existing.id == event.id) {
            Some(existing) => *existing = event,
            None => events.push(event),
        }
        self.rebuild_structure(Some(guild_id));
        Change::Structure
    }

    pub(super) fn scheduled_event_delete(&mut self, event: &model::ScheduledEvent) -> Change {
        let Some(guild_id) = event.guild_id else {
            return Change::None;
        };
        let Some(events) = self
            .source_guild_mut(guild_id)
            .and_then(|guild| guild.guild_scheduled_events.as_mut())
        else {
            return Change::None;
        };
        events.retain(|existing| existing.id != event.id);
        self.rebuild_structure(Some(guild_id));
        Change::Structure
    }

    pub(super) fn guild_delete(&mut self, id: u64) -> Change {
        self.voice_start_times.retain(|(guild, _), _| *guild != id);
        self.social.guilds.remove(&id);
        self.rail_order.retain(|guild| *guild != id);
        if let Some(source) = &mut self.source {
            source.guilds.retain(|g| g.id != id);
        }
        self.rebuild_structure(Some(id));
        Change::Structure
    }
}
