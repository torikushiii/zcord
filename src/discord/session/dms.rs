use super::super::*;

impl Session {
    pub async fn close_dm(&self, channel_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.close_dm(channel_id).await)
            .await
    }

    pub async fn leave_group(&self, channel_id: u64, silent: bool) -> Result<(), Error> {
        self.rest(async |rest| rest.leave_group(channel_id, silent).await)
            .await
    }

    pub async fn block_user(&self, user_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.block_user(user_id).await).await
    }

    pub async fn unblock_user(&self, user_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.unblock_user(user_id).await)
            .await
    }

    pub async fn ignore_user(&self, user_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.ignore_user(user_id).await)
            .await
    }

    pub async fn unignore_user(&self, user_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.unignore_user(user_id).await)
            .await
    }

    pub async fn set_friend_nickname(
        &self,
        user_id: u64,
        nickname: Option<&str>,
    ) -> Result<(), Error> {
        self.rest(async |rest| rest.set_friend_nickname(user_id, nickname).await)
            .await
    }

    pub async fn set_dm_mute(&self, channel_id: u64, mute: Option<i64>) -> Result<(), Error> {
        self.rest(async |rest| rest.set_dm_mute(channel_id, mute).await)
            .await
    }

    pub async fn create_group_dm(&self, recipients: &[u64]) -> Result<model::Channel, Error> {
        self.rest(async |rest| rest.create_group_dm(recipients).await)
            .await
    }

    pub async fn add_group_recipient(&self, channel_id: u64, user_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.add_group_recipient(channel_id, user_id).await)
            .await
    }

    pub async fn remove_group_recipient(&self, channel_id: u64, user_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.remove_group_recipient(channel_id, user_id).await)
            .await
    }
}
