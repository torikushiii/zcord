use super::super::*;

impl Session {
    /// Explicit metadata fetch; caching and click/hover policy belong to the caller.
    pub async fn sticker_details(&self, id: u64) -> Result<model::StickerDetails, Error> {
        self.rest(async |rest| rest.fetch_sticker_details(id).await)
            .await
    }

    /// Resolves a sticker's external guild without changing joined-guild state.
    pub async fn sticker_guild(&self, sticker_id: u64) -> Result<model::StickerGuild, Error> {
        self.rest(async |rest| rest.fetch_sticker_guild(sticker_id).await)
            .await
    }

    /// Explicit fallback for missing or truncated discovery catalogs; does not join.
    pub async fn sticker_guild_preview(&self, guild_id: u64) -> Result<model::StickerGuild, Error> {
        self.rest(async |rest| rest.fetch_sticker_guild_preview(guild_id).await)
            .await
    }
}
