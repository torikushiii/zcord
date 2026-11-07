use super::super::*;

impl Session {
    pub async fn saved_messages(&self) -> Result<model::SavedMessages, Error> {
        self.rest(async |rest| rest.saved_messages().await).await
    }

    pub async fn bookmarks(&self, before: Option<String>) -> Result<model::BookmarkPage, Error> {
        self.rest(async |rest| rest.bookmarks(before.as_deref()).await)
            .await
    }

    pub async fn save_message(
        &self,
        channel_id: u64,
        message_id: u64,
        due_at: Option<String>,
    ) -> Result<model::SavedMessage, Error> {
        self.rest(async |rest| {
            rest.save_message(channel_id, message_id, due_at.as_deref())
                .await
        })
        .await
    }

    pub async fn unsave_message(&self, channel_id: u64, message_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.unsave_message(channel_id, message_id).await)
            .await
    }
}
