use super::super::*;

impl Session {
    pub async fn pinned_messages(
        &self,
        channel_id: u64,
        before: Option<String>,
    ) -> Result<PinPage, Error> {
        self.rest(async |rest| rest.pinned_messages(channel_id, before.as_deref()).await)
            .await
    }

    pub async fn pin_message(&self, channel_id: u64, message_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.pin_message(channel_id, message_id).await)
            .await
    }

    pub async fn unpin_message(&self, channel_id: u64, message_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.unpin_message(channel_id, message_id).await)
            .await
    }

    pub async fn ack_pins(&self, channel_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.ack_pins(channel_id).await)
            .await
    }
}
