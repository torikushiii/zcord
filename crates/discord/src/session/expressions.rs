use super::super::*;

impl Session {
    pub async fn sticker_details(&self, id: u64) -> Result<model::StickerDetails, Error> {
        self.rest(async |rest| rest.fetch_sticker_details(id).await)
            .await
    }

    pub async fn sticker_guild(&self, sticker_id: u64) -> Result<model::StickerGuild, Error> {
        self.rest(async |rest| rest.fetch_sticker_guild(sticker_id).await)
            .await
    }

    pub async fn sticker_guild_preview(&self, guild_id: u64) -> Result<model::StickerGuild, Error> {
        self.rest(async |rest| rest.fetch_sticker_guild_preview(guild_id).await)
            .await
    }
}
