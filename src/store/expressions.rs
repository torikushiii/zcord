use super::*;

impl Store {
    /// Resolves only exact IDs in the current joined-guild emoji catalogs.
    pub fn emoji_origin(&self, id: u64) -> Option<(&Guild, &CustomEmoji)> {
        self.guilds.iter().find_map(|guild| {
            guild
                .emojis
                .iter()
                .find(|emoji| emoji.id == id)
                .map(|emoji| (guild, emoji))
        })
    }

    /// Missing catalog metadata is unknown, not evidence of a different origin.
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

    /// Joined-guild source catalog: `None` is unknown, `Some(&[])` is known empty.
    pub fn guild_stickers(&self, guild_id: u64) -> Option<&[model::StickerDetails]> {
        self.source
            .as_ref()?
            .guilds
            .iter()
            .find(|guild| guild.id == guild_id)?
            .stickers
            .as_deref()
    }

    /// `None` means the source guild or its feature list is unknown.
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
}
