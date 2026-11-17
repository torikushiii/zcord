use super::super::*;

impl Session {
    pub async fn send_friend_request(&self, username: &str) -> Result<(), Error> {
        self.rest(async |rest| rest.send_friend_request(username).await)
            .await
    }

    pub async fn add_friend(
        &self,
        user_id: u64,
        location: model::RelationshipLocation,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
    ) -> Result<(), Error> {
        self.rest(async |rest| {
            rest.add_friend(user_id, location, guild_id, channel_id)
                .await
        })
        .await
    }

    pub async fn accept_friend_request(
        &self,
        user_id: u64,
        location: model::RelationshipLocation,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
    ) -> Result<(), Error> {
        self.rest(async |rest| {
            rest.accept_friend_request(user_id, location, guild_id, channel_id)
                .await
        })
        .await
    }

    pub async fn delete_relationship(
        &self,
        user_id: u64,
        location: model::RelationshipLocation,
    ) -> Result<(), Error> {
        self.rest(async |rest| rest.delete_relationship(user_id, location).await)
            .await
    }

    pub async fn friend_suggestions(&self) -> Result<Vec<model::FriendSuggestion>, Error> {
        self.rest(async |rest| rest.fetch_friend_suggestions().await)
            .await
    }

    pub async fn open_dm(&self, user_id: u64) -> Result<model::Channel, Error> {
        self.rest(async |rest| rest.open_dm(user_id).await).await
    }

    pub async fn accept_message_request(&self, channel_id: u64) -> Result<model::Channel, Error> {
        self.rest(async |rest| rest.accept_message_request(channel_id).await)
            .await
    }

    pub async fn reject_message_request(&self, channel_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.reject_message_request(channel_id).await)
            .await
    }

    pub async fn message_request_previews(
        &self,
        channel_ids: Vec<u64>,
    ) -> Result<Vec<model::MessageRequestPreview>, Error> {
        self.rest(async |rest| rest.message_request_previews(&channel_ids).await)
            .await
    }
}
