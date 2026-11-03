use super::super::*;

impl Session {
    pub async fn set_guild_mute(&self, guild_id: u64, mute: Option<i64>) -> Result<(), Error> {
        self.rest(async |rest| rest.set_guild_mute(guild_id, mute).await)
            .await
    }

    pub async fn set_guild_notifications(
        &self,
        guild_id: u64,
        level: model::NotificationLevel,
    ) -> Result<(), Error> {
        self.rest(async |rest| rest.set_guild_notifications(guild_id, level).await)
            .await
    }

    pub async fn set_guild_notification_flag(
        &self,
        guild_id: u64,
        flag: model::GuildNotificationFlag,
        on: bool,
    ) -> Result<(), Error> {
        self.rest(async |rest| rest.set_guild_notification_flag(guild_id, flag, on).await)
            .await
    }
}
