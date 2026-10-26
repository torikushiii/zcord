use super::super::*;
use crate::discord::rest::Anchor;

impl Session {
    pub async fn fetch_messages(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<model::Message>, Error> {
        self.rest(async |rest| {
            rest.fetch_messages(guild_id, channel_id, before, limit)
                .await
        })
        .await
    }

    pub async fn ack_bulk(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        acks: Vec<(u64, u64)>,
    ) -> Result<(), Error> {
        self.rest(async |rest| rest.ack_bulk(guild_id, channel_id, &acks).await)
            .await
    }

    pub async fn ack_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        last_viewed: u32,
    ) -> Result<(), Error> {
        self.rest(async |rest| {
            rest.ack_message(guild_id, channel_id, message_id, last_viewed)
                .await
        })
        .await
    }

    pub fn send_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message: OutgoingMessage,
    ) -> impl Future<Output = Result<model::Message, Error>> + Send + 'static {
        // The ticket is taken here, before spawning, so sends keep composer order.
        let (done, turn) = tokio::sync::oneshot::channel::<()>();
        let previous = self
            .inner
            .sends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(channel_id, turn);
        let session = self.clone();
        async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let result = session
                .rest(async |rest| rest.send_message(guild_id, channel_id, &message).await)
                .await;
            drop(done);
            result
        }
    }

    pub async fn edit_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        content: String,
    ) -> Result<model::MessageUpdate, Error> {
        self.rest(async |rest| {
            rest.edit_message(guild_id, channel_id, message_id, &content)
                .await
        })
        .await
    }

    pub async fn delete_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
    ) -> Result<(), Error> {
        self.rest(async |rest| rest.delete_message(guild_id, channel_id, message_id).await)
            .await
    }

    pub async fn trigger_typing(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
    ) -> Result<bool, Error> {
        self.rest(async |rest| rest.trigger_typing(guild_id, channel_id).await)
            .await
    }

    pub async fn fetch_forum_posts(
        &self,
        guild_id: Option<u64>,
        forum_id: u64,
        offset: usize,
    ) -> Result<model::ForumSearch, Error> {
        self.rest(async |rest| rest.fetch_forum_posts(guild_id, forum_id, offset).await)
            .await
    }

    pub async fn fetch_messages_after(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        after: u64,
        limit: usize,
    ) -> Result<Vec<model::Message>, Error> {
        self.rest(async |rest| {
            rest.fetch_messages_anchor(guild_id, channel_id, Anchor::After(after), limit)
                .await
        })
        .await
    }

    pub async fn fetch_messages_around(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        around: u64,
        limit: usize,
    ) -> Result<Vec<model::Message>, Error> {
        self.rest(async |rest| {
            rest.fetch_messages_anchor(guild_id, channel_id, Anchor::Around(around), limit)
                .await
        })
        .await
    }

    pub async fn add_reaction(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        emoji: &str,
    ) -> Result<(), Error> {
        self.rest(async |rest| {
            rest.add_reaction(guild_id, channel_id, message_id, emoji)
                .await
        })
        .await
    }

    pub async fn remove_reaction(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        emoji: &str,
    ) -> Result<(), Error> {
        self.rest(async |rest| {
            rest.remove_reaction(guild_id, channel_id, message_id, emoji)
                .await
        })
        .await
    }
}
