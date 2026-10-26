use super::super::*;

impl Session {
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
