use super::super::*;

impl Session {
    pub async fn mentions(
        &self,
        before: Option<u64>,
        roles: bool,
        everyone: bool,
    ) -> Result<Vec<model::Message>, Error> {
        self.rest(async |rest| rest.mentions(before, roles, everyone).await)
            .await
    }

    pub async fn delete_mention(&self, message_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.delete_mention(message_id).await)
            .await
    }

    pub async fn scheduled_messages(&self) -> Result<Vec<model::ScheduledMessage>, Error> {
        self.rest(async |rest| rest.scheduled_messages().await)
            .await
    }

    pub async fn cancel_scheduled(&self, scheduled_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.cancel_scheduled(scheduled_id).await)
            .await
    }

    pub async fn send_scheduled_now(&self, scheduled_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.send_scheduled_now(scheduled_id).await)
            .await
    }

    pub async fn update_scheduled(
        &self,
        scheduled_id: u64,
        send_at: Option<String>,
        content: Option<(String, u64)>,
    ) -> Result<model::ScheduledMessage, Error> {
        self.rest(async |rest| {
            rest.update_scheduled(
                scheduled_id,
                send_at.as_deref(),
                content
                    .as_ref()
                    .map(|(content, flags)| (content.as_str(), *flags)),
            )
            .await
        })
        .await
    }
}
