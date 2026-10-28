use super::*;

impl Rest {
    pub async fn fetch_sticker_details(&self, id: u64) -> Result<model::StickerDetails, Error> {
        let url = format!("https://discord.com/api/v9/stickers/{id}");
        let response = self.send(|| self.client.get(&url)).await?;
        let details: model::StickerDetails = response.json().await.map_err(|_| Error::Decode)?;
        if details.id != id {
            return Err(Error::Decode);
        }
        Ok(details)
    }

    pub async fn fetch_sticker_guild(&self, sticker_id: u64) -> Result<model::StickerGuild, Error> {
        let url = format!("https://discord.com/api/v9/stickers/{sticker_id}/guild");
        let response = self.send(|| self.client.get(&url)).await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn fetch_sticker_guild_preview(
        &self,
        guild_id: u64,
    ) -> Result<model::StickerGuild, Error> {
        let url = format!("https://discord.com/api/v9/guilds/{guild_id}/preview");
        let response = self.send(|| self.client.get(&url)).await?;
        let guild: model::StickerGuild = response.json().await.map_err(|_| Error::Decode)?;
        if guild.id != guild_id {
            return Err(Error::Decode);
        }
        Ok(guild)
    }
}
