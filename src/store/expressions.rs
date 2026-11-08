use super::*;

impl Store {
    pub fn emoji_origin(&self, id: u64) -> Option<(&Guild, &CustomEmoji)> {
        self.guilds.iter().find_map(|guild| {
            guild
                .emojis
                .iter()
                .find(|emoji| emoji.id == id)
                .map(|emoji| (guild, emoji))
        })
    }

    pub fn sticker_origin(&self, id: u64) -> Option<&Guild> {
        let source = self.source.as_ref()?;
        let guild = source.guilds.iter().find(|guild| {
            guild
                .stickers
                .iter()
                .flatten()
                .any(|sticker| sticker.id == id)
        })?;
        self.guild_by_id(guild.id)
    }

    pub fn guild_stickers(&self, guild_id: u64) -> Option<&[model::StickerDetails]> {
        self.source
            .as_ref()?
            .guilds
            .iter()
            .find(|guild| guild.id == guild_id)?
            .stickers
            .as_deref()
    }

    pub fn guild_discoverable(&self, id: u64) -> Option<bool> {
        let guild = self
            .source
            .as_ref()?
            .guilds
            .iter()
            .find(|guild| guild.id == id)?;
        Some(
            guild
                .features()?
                .iter()
                .any(|feature| feature == "DISCOVERABLE"),
        )
    }

    pub fn sticker(&self, id: u64) -> Option<Sticker> {
        let details = self
            .source
            .as_ref()?
            .guilds
            .iter()
            .flat_map(|guild| guild.stickers.iter().flatten())
            .find(|sticker| sticker.id == id)?;
        Some(Sticker {
            id: details.id,
            name: details.name.clone(),
            format: details.format_type,
        })
    }

    pub fn sticker_sections(&self, guild_id: Option<u64>, channel_id: u64) -> Vec<StickerSection> {
        let mut ids = Vec::new();
        if let Some(id) = guild_id {
            ids.push(id);
        }
        for entry in &self.rail {
            let listed = match entry {
                RailEntry::Guild(id) => std::slice::from_ref(id),
                RailEntry::Folder(folder) => &folder.guild_ids,
            };
            for id in listed {
                if !ids.contains(id) {
                    ids.push(*id);
                }
            }
        }
        let sendable = self.can_send(channel_id);
        let nitro = self.has_nitro();
        let dm = matches!(
            self.source_channels.get(&channel_id),
            Some(SourceChannel::Private(_))
        );
        let external = self
            .permissions(channel_id)
            .is_some_and(|permissions| permissions & USE_EXTERNAL_STICKERS != 0);
        ids.into_iter()
            .filter_map(|id| {
                let guild = self.guild_by_id(id)?;
                let stickers: Vec<PickerSticker> = self
                    .guild_stickers(id)?
                    .iter()
                    .filter(|sticker| sticker.available != Some(false))
                    .map(|sticker| {
                        let current = guild_id == Some(id);
                        PickerSticker {
                            id: sticker.id,
                            name: sticker.name.clone(),
                            tags: sticker.tags.clone().unwrap_or_default(),
                            description: sticker.description.clone(),
                            format_type: sticker.format_type,
                            usable: sendable && (current || (nitro && (dm || external))),
                        }
                    })
                    .collect();
                if stickers.is_empty() {
                    return None;
                }
                Some(StickerSection {
                    guild_id: id,
                    name: guild.name.clone(),
                    icon: guild.icon_spec(),
                    stickers,
                })
            })
            .collect()
    }
}

pub struct StickerSection {
    pub guild_id: u64,
    pub name: String,
    pub icon: Option<ImageSpec>,
    pub stickers: Vec<PickerSticker>,
}

pub struct PickerSticker {
    pub id: u64,
    pub name: String,
    pub tags: String,
    pub description: Option<String>,
    pub format_type: u8,
    pub usable: bool,
}

pub fn sticker_matches(sticker: &PickerSticker, query: &str) -> bool {
    let query = query.to_lowercase();
    if sticker.name.to_lowercase().contains(&query) {
        return true;
    }
    sticker
        .tags
        .split(',')
        .any(|tag| tag.trim().to_lowercase().contains(&query))
}

impl From<&PickerSticker> for Sticker {
    fn from(sticker: &PickerSticker) -> Self {
        Self {
            id: sticker.id,
            name: sticker.name.clone(),
            format: sticker.format_type,
        }
    }
}

const USE_EXTERNAL_STICKERS: u64 = 1 << 37;
