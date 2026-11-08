use super::super::*;
use crate::rest::{Anchor, put_upload};

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

    pub async fn forward_message(
        &self,
        source_guild: Option<u64>,
        source_channel: u64,
        message_id: u64,
        target_guild: Option<u64>,
        target_channel: u64,
        nonce: u64,
    ) -> Result<model::Message, Error> {
        self.rest(async |rest| {
            rest.forward_message(
                source_guild,
                source_channel,
                message_id,
                target_guild,
                target_channel,
                nonce,
            )
            .await
        })
        .await
    }

    pub async fn send_forward_note(
        &self,
        target_guild: Option<u64>,
        target_channel: u64,
        content: &str,
        nonce: u64,
    ) -> Result<model::Message, Error> {
        self.rest(async |rest| {
            rest.send_forward_note(target_guild, target_channel, content, nonce)
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

    pub async fn ack_manual(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        mention_count: u32,
    ) -> Result<(), Error> {
        self.rest(async |rest| {
            rest.ack_manual(guild_id, channel_id, message_id, mention_count)
                .await
        })
        .await
    }

    fn begin_send(
        &self,
        channel_id: u64,
    ) -> (
        Session,
        Option<tokio::sync::oneshot::Receiver<()>>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let (done, turn) = tokio::sync::oneshot::channel::<()>();
        let previous = self
            .inner
            .sends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(channel_id, turn);
        (self.clone(), previous, done)
    }

    pub fn send_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message: OutgoingMessage,
    ) -> impl Future<Output = Result<model::Message, Error>> + Send + 'static {
        let (session, previous, done) = self.begin_send(channel_id);
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

    pub fn send_with_files(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        mut message: OutgoingMessage,
        files: Vec<UploadFile>,
        progress: Arc<UploadProgress>,
    ) -> impl Future<Output = Result<model::Message, Error>> + Send + 'static {
        let (session, previous, done) = self.begin_send(channel_id);
        async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let result = session
                .send_with_files_inner(guild_id, channel_id, &mut message, &files, &progress)
                .await;
            drop(done);
            result
        }
    }

    async fn send_with_files_inner(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message: &mut OutgoingMessage,
        files: &[UploadFile],
        progress: &Arc<UploadProgress>,
    ) -> Result<model::Message, Error> {
        let upload_client = self
            .inner
            .rest
            .as_ref()
            .map_err(Clone::clone)?
            .upload_client
            .clone();
        let created = std::sync::Mutex::new(Vec::new());
        let uploads = files.iter().map(|file| async {
            let slots = self
                .rest(async |rest| {
                    rest.create_uploads(guild_id, channel_id, std::slice::from_ref(file))
                        .await
                })
                .await?;
            let slot = slots.into_iter().next().ok_or(Error::Decode)?;
            created.lock().unwrap().push(slot.clone());
            put_upload(&upload_client, &slot.upload_url, file, progress).await?;
            Ok::<_, Error>(slot)
        });
        let uploaded = match futures::future::try_join_all(uploads).await {
            Ok(uploaded) => uploaded,
            Err(error) => {
                let created = std::mem::take(&mut *created.lock().unwrap());
                self.discard_uploads(&created).await;
                return Err(error);
            }
        };
        message.attachments = files
            .iter()
            .zip(&uploaded)
            .map(|(file, slot)| OutgoingAttachment {
                filename: file.filename.clone(),
                uploaded_filename: slot.upload_filename.clone(),
                content_type: file.content_type.clone(),
                spoiler: file.spoiler,
            })
            .collect();
        match self
            .rest(async |rest| rest.send_message(guild_id, channel_id, message).await)
            .await
        {
            Ok(message) => Ok(message),
            Err(error) => {
                self.discard_uploads(&uploaded).await;
                Err(error)
            }
        }
    }

    async fn discard_uploads(&self, uploads: &[UploadTarget]) {
        for upload in uploads {
            let _ = self
                .rest(async |rest| rest.delete_upload(&upload.upload_filename).await)
                .await;
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
        query: ForumQuery,
    ) -> Result<model::ForumSearch, Error> {
        self.rest(async |rest| rest.fetch_forum_posts(guild_id, forum_id, &query).await)
            .await
    }

    pub async fn search_messages(
        &self,
        scope: SearchScope,
        query: &SearchQuery,
    ) -> Result<SearchOutcome, Error> {
        self.rest(async |rest| rest.search_messages(scope, query).await)
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

    pub async fn reaction_users(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        emoji: &str,
        page: model::ReactorPage,
    ) -> Result<Vec<model::User>, Error> {
        self.rest(async |rest| {
            rest.reaction_users(guild_id, channel_id, message_id, emoji, page)
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
