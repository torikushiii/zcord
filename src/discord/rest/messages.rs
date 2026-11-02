use super::*;
use futures::Stream;
use reqwest::{
    Body,
    header::{CONTENT_LENGTH, CONTENT_TYPE},
};
use serde::Deserialize;
use std::sync::{Arc, atomic::Ordering};
use tokio::io::AsyncReadExt;

const UPLOAD_CHUNK: usize = 64 * 1024;
const UPLOAD_STALL: Duration = Duration::from_secs(60);
const UPLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CHAT_INPUT_CONTEXT: &str = "eyJsb2NhdGlvbiI6ImNoYXRfaW5wdXQifQ==";
const FORWARD_CONTEXT: &str = "eyJsb2NhdGlvbiI6ImZvcndhcmRpbmcifQ==";

pub(super) fn history_query(anchor: Anchor, limit: usize) -> Vec<(&'static str, String)> {
    let mut query = Vec::new();
    match anchor {
        Anchor::Before(id) => query.push(("before", id.to_string())),
        Anchor::After(id) => query.push(("after", id.to_string())),
        Anchor::Around(id) => query.push(("around", id.to_string())),
        Anchor::Latest => {}
    }
    query.push(("limit", limit.to_string()));
    query
}

// `archived` is omitted on purpose: `true` restricts the search to archived posts only, and
// leaving it out searches active and archived posts together.
pub(super) fn forum_search_query(offset: usize) -> Vec<(&'static str, String)> {
    vec![
        ("sort_by", "last_message_time".to_owned()),
        ("sort_order", "desc".to_owned()),
        ("limit", "25".to_owned()),
        ("tag_setting", "match_some".to_owned()),
        ("offset", offset.to_string()),
    ]
}

#[derive(Serialize)]
pub(super) struct CreateMessage<'a> {
    mobile_network_type: &'static str,
    content: &'a str,
    nonce: String,
    tts: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_reference: Option<MessageReferenceBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_mentions: Option<AllowedMentions>,
    flags: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<AttachmentBody<'a>>,
}

#[derive(Serialize)]
struct AttachmentBody<'a> {
    id: String,
    description: &'a str,
    filename: &'a str,
    uploaded_filename: &'a str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    is_spoiler: bool,
    original_content_type: &'a str,
}

#[derive(Serialize)]
struct MessageReferenceBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    guild_id: Option<String>,
    channel_id: String,
    message_id: String,
    #[serde(rename = "type")]
    kind: u8,
}

// `parse` must stay explicit: with any allowed_mentions object present, an omitted
// `parse` suppresses content mentions, so a non-pinging reply would stop pinging @user.
#[derive(Serialize)]
struct AllowedMentions {
    parse: [&'static str; 3],
    replied_user: bool,
}

#[derive(Serialize)]
struct EditMessage<'a> {
    content: &'a str,
}

pub(super) fn message_body<'a>(
    guild_id: Option<u64>,
    channel_id: u64,
    message: &'a OutgoingMessage,
) -> CreateMessage<'a> {
    CreateMessage {
        mobile_network_type: "unknown",
        content: &message.content,
        nonce: message.nonce.to_string(),
        tts: false,
        message_reference: message.reply.as_ref().map(|reply| MessageReferenceBody {
            guild_id: guild_id.map(|id| id.to_string()),
            channel_id: channel_id.to_string(),
            message_id: reply.message_id.to_string(),
            kind: 0,
        }),
        allowed_mentions: message
            .reply
            .as_ref()
            .filter(|reply| !reply.mention)
            .map(|_| AllowedMentions {
                parse: ["users", "roles", "everyone"],
                replied_user: false,
            }),
        flags: 0,
        attachments: message
            .attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| AttachmentBody {
                id: index.to_string(),
                description: "",
                filename: &attachment.filename,
                uploaded_filename: &attachment.uploaded_filename,
                is_spoiler: attachment.spoiler,
                original_content_type: &attachment.content_type,
            })
            .collect(),
    }
}

#[derive(Serialize)]
pub(super) struct ForwardMessage {
    content: &'static str,
    nonce: String,
    tts: bool,
    flags: u64,
    mobile_network_type: &'static str,
    message_reference: ForwardReference,
}

#[derive(Serialize)]
struct ForwardReference {
    guild_id: Option<String>,
    channel_id: String,
    message_id: String,
    #[serde(rename = "type")]
    kind: u8,
}

pub(super) fn forward_body(
    source_guild: Option<u64>,
    source_channel: u64,
    message_id: u64,
    nonce: u64,
) -> ForwardMessage {
    ForwardMessage {
        content: "",
        nonce: nonce.to_string(),
        tts: false,
        flags: 0,
        mobile_network_type: "unknown",
        message_reference: ForwardReference {
            guild_id: source_guild.map(|id| id.to_string()),
            channel_id: source_channel.to_string(),
            message_id: message_id.to_string(),
            kind: 1,
        },
    }
}

#[derive(Serialize)]
pub(super) struct CreateUploads<'a> {
    files: Vec<UploadFileBody<'a>>,
}

#[derive(Serialize)]
struct UploadFileBody<'a> {
    filename: &'a str,
    file_size: u64,
    id: String,
    is_clip: bool,
    original_content_type: &'a str,
}

#[derive(Deserialize)]
struct UploadsResponse {
    attachments: Vec<UploadSlot>,
}

#[derive(Deserialize)]
struct UploadSlot {
    id: u64,
    upload_url: String,
    upload_filename: String,
}

pub(super) fn uploads_body(files: &[UploadFile]) -> CreateUploads<'_> {
    CreateUploads {
        files: files
            .iter()
            .enumerate()
            .map(|(index, file)| UploadFileBody {
                filename: &file.filename,
                file_size: file.size,
                id: index.to_string(),
                is_clip: false,
                original_content_type: &file.content_type,
            })
            .collect(),
    }
}

// The whole `<uuid>/<name>` is one path segment, so its slash is encoded too.
pub(super) fn delete_upload_url(uploaded_filename: &str) -> String {
    format!(
        "https://discord.com/api/v9/attachments/{}",
        percent_encode(uploaded_filename)
    )
}

pub(crate) fn upload_client() -> Result<Client, Error> {
    Client::builder()
        .connect_timeout(UPLOAD_CONNECT_TIMEOUT)
        .build()
        .map_err(|_| Error::Network)
}

pub(super) fn parse_uploads(body: &str) -> Result<Vec<UploadTarget>, Error> {
    let response: UploadsResponse = serde_json::from_str(body).map_err(|_| Error::Decode)?;
    Ok(response
        .attachments
        .into_iter()
        .map(|slot| UploadTarget {
            id: slot.id,
            upload_url: slot.upload_url,
            upload_filename: slot.upload_filename,
        })
        .collect())
}

pub(super) fn put_request(
    client: &Client,
    target_url: &str,
    file: &UploadFile,
    body: Body,
) -> RequestBuilder {
    client
        .put(target_url)
        .header(CONTENT_TYPE, "application/octet-stream")
        .header(CONTENT_LENGTH, file.size)
        .body(body)
}

pub(super) fn upload_stream(
    file: tokio::fs::File,
    progress: Arc<UploadProgress>,
) -> impl Stream<Item = Result<Vec<u8>, Error>> + Send + 'static {
    futures::stream::try_unfold((file, progress), |(mut file, progress)| async move {
        if progress.cancelled.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        let mut buffer = vec![0; UPLOAD_CHUNK];
        let read = file.read(&mut buffer).await.map_err(|_| Error::Network)?;
        if read == 0 {
            return Ok(None);
        }
        buffer.truncate(read);
        progress.sent.fetch_add(read as u64, Ordering::AcqRel);
        Ok(Some((buffer, (file, progress))))
    })
}

pub(crate) async fn put_upload(
    client: &Client,
    target_url: &str,
    file: &UploadFile,
    progress: &Arc<UploadProgress>,
) -> Result<(), Error> {
    put_upload_watched(client, target_url, file, progress, UPLOAD_STALL).await
}

pub(super) async fn put_upload_watched(
    client: &Client,
    target_url: &str,
    file: &UploadFile,
    progress: &Arc<UploadProgress>,
    stall: Duration,
) -> Result<(), Error> {
    let handle = tokio::fs::File::open(&file.path)
        .await
        .map_err(|_| Error::Network)?;
    let body = Body::wrap_stream(upload_stream(handle, progress.clone()));
    let send = put_request(client, target_url, file, body).send();
    tokio::pin!(send);
    // A stalled socket stops polling the body, so the body can't notice a cancel or a stall.
    let mut watch = tokio::time::interval((stall / 4).min(Duration::from_secs(1)));
    let mut seen = 0;
    let mut moved = tokio::time::Instant::now();
    let result = loop {
        tokio::select! {
            result = &mut send => break result,
            _ = watch.tick() => {
                if progress.cancelled.load(Ordering::Acquire) {
                    return Err(Error::Cancelled);
                }
                let sent = progress.sent.load(Ordering::Acquire);
                if sent != seen {
                    seen = sent;
                    moved = tokio::time::Instant::now();
                } else if moved.elapsed() >= stall {
                    return Err(Error::Network);
                }
            }
        }
    };
    let cancelled = progress.cancelled.load(Ordering::Acquire);
    match result {
        Ok(response) if response.status().is_success() => Ok(()),
        Ok(_) if cancelled => Err(Error::Cancelled),
        Ok(response) => Err(Error::Http(response.status().as_u16())),
        Err(_) if cancelled => Err(Error::Cancelled),
        Err(_) => Err(Error::Network),
    }
}

#[derive(Serialize)]
struct AckMessage {
    token: Option<String>,
    last_viewed: u32,
}

#[derive(Serialize)]
struct AckManual {
    manual: bool,
    mention_count: u32,
}

#[derive(Serialize)]
struct AckBulk {
    read_states: Vec<ReadStateAck>,
}

#[derive(Serialize)]
struct ReadStateAck {
    channel_id: String,
    message_id: String,
    read_state_type: u8,
}

pub(super) fn reaction_url(channel_id: u64, message_id: u64, emoji: &str) -> String {
    format!(
        "https://discord.com/api/v9/channels/{channel_id}/messages/{message_id}/reactions/{}/@me",
        percent_encode(emoji)
    )
}

impl Rest {
    pub async fn fetch_messages(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<model::Message>, Error> {
        let anchor = before.map_or(Anchor::Latest, Anchor::Before);
        self.fetch_messages_anchor(guild_id, channel_id, anchor, limit)
            .await
    }

    pub async fn fetch_forum_posts(
        &self,
        guild_id: Option<u64>,
        forum_id: u64,
        offset: usize,
    ) -> Result<model::ForumSearch, Error> {
        let referer = referer(guild_id, forum_id);
        let url = format!("https://discord.com/api/v9/channels/{forum_id}/threads/search");
        let query = forum_search_query(offset);
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .query(&query)
                    .header(REFERER, &referer)
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub(in crate::discord) async fn fetch_messages_anchor(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        anchor: Anchor,
        limit: usize,
    ) -> Result<Vec<model::Message>, Error> {
        let referer = referer(guild_id, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/messages");
        let query = history_query(anchor, limit);
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .query(&query)
                    .header(REFERER, &referer)
            })
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        if !value.is_array() {
            return Err(Error::Decode);
        }
        let mut messages: Vec<model::Message> = model::parse_entries(value);
        messages.retain(|m| m.channel_id == channel_id);
        messages.sort_by_key(|m| m.id);
        messages.dedup_by_key(|m| m.id);
        Ok(messages)
    }

    pub(in crate::discord) async fn add_reaction(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        emoji: &str,
    ) -> Result<(), Error> {
        let referer = referer(guild_id, channel_id);
        let url = reaction_url(channel_id, message_id, emoji);
        self.send(|| {
            self.client
                .put(&url)
                .header(REFERER, &referer)
                .query(&[("location", "Message"), ("type", "0")])
        })
        .await?;
        Ok(())
    }

    pub(in crate::discord) async fn remove_reaction(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        emoji: &str,
    ) -> Result<(), Error> {
        let referer = referer(guild_id, channel_id);
        let url = reaction_url(channel_id, message_id, emoji);
        self.send(|| {
            self.client
                .delete(&url)
                .header(REFERER, &referer)
                .query(&[("location", "Message"), ("type", "0")])
        })
        .await?;
        Ok(())
    }

    pub async fn ack_bulk(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        acks: &[(u64, u64)],
    ) -> Result<(), Error> {
        let referer = referer(guild_id, channel_id);
        for chunk in acks.chunks(ACK_BULK_LIMIT) {
            let read_states: Vec<_> = chunk
                .iter()
                .map(|(channel_id, message_id)| ReadStateAck {
                    channel_id: channel_id.to_string(),
                    message_id: message_id.to_string(),
                    read_state_type: 0,
                })
                .collect();
            let body = AckBulk { read_states };
            self.send(|| {
                self.client
                    .post("https://discord.com/api/v9/read-states/ack-bulk")
                    .header(REFERER, &referer)
                    .json(&body)
            })
            .await?;
        }
        Ok(())
    }

    pub async fn create_uploads(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        files: &[UploadFile],
    ) -> Result<Vec<UploadTarget>, Error> {
        let referer = referer(guild_id, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/attachments");
        let body = uploads_body(files);
        let response = self
            .send(|| self.client.post(&url).header(REFERER, &referer).json(&body))
            .await?;
        let body = response.text().await.map_err(|_| Error::Decode)?;
        parse_uploads(&body)
    }

    pub async fn delete_upload(&self, uploaded_filename: &str) -> Result<(), Error> {
        let url = delete_upload_url(uploaded_filename);
        self.send(|| self.client.delete(&url)).await?;
        Ok(())
    }

    pub async fn send_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message: &OutgoingMessage,
    ) -> Result<model::Message, Error> {
        let referer = referer(guild_id, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/messages");
        let body = message_body(guild_id, channel_id, message);
        let response = self
            .send(|| {
                self.client
                    .post(&url)
                    .header(REFERER, &referer)
                    .header("x-context-properties", CHAT_INPUT_CONTEXT)
                    .json(&body)
            })
            .await?;
        response
            .json::<model::Message>()
            .await
            .map_err(|_| Error::Decode)
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
        let referer = referer(target_guild, target_channel);
        let url = format!("https://discord.com/api/v9/channels/{target_channel}/messages");
        let body = forward_body(source_guild, source_channel, message_id, nonce);
        let response = self
            .send(|| {
                self.client
                    .post(&url)
                    .header(REFERER, &referer)
                    .header("x-context-properties", FORWARD_CONTEXT)
                    .json(&body)
            })
            .await?;
        response
            .json::<model::Message>()
            .await
            .map_err(|_| Error::Decode)
    }

    pub async fn send_forward_note(
        &self,
        target_guild: Option<u64>,
        target_channel: u64,
        content: &str,
        nonce: u64,
    ) -> Result<model::Message, Error> {
        let referer = referer(target_guild, target_channel);
        let url = format!("https://discord.com/api/v9/channels/{target_channel}/messages");
        let message = OutgoingMessage {
            content: content.to_owned(),
            nonce,
            reply: None,
            attachments: Vec::new(),
        };
        let body = message_body(target_guild, target_channel, &message);
        let response = self
            .send(|| {
                self.client
                    .post(&url)
                    .header(REFERER, &referer)
                    .header("x-context-properties", FORWARD_CONTEXT)
                    .json(&body)
            })
            .await?;
        response
            .json::<model::Message>()
            .await
            .map_err(|_| Error::Decode)
    }

    pub async fn edit_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        content: &str,
    ) -> Result<model::MessageUpdate, Error> {
        let referer = referer(guild_id, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/messages/{message_id}");
        let body = EditMessage { content };
        let response = self
            .send(|| {
                self.client
                    .patch(&url)
                    .header(REFERER, &referer)
                    .json(&body)
            })
            .await?;
        response
            .json::<model::MessageUpdate>()
            .await
            .map_err(|_| Error::Decode)
    }

    pub async fn delete_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
    ) -> Result<(), Error> {
        let referer = referer(guild_id, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/messages/{message_id}");
        self.send(|| self.client.delete(&url).header(REFERER, &referer))
            .await?;
        Ok(())
    }

    pub async fn trigger_typing(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
    ) -> Result<bool, Error> {
        let referer = referer(guild_id, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/typing");
        let request = self
            .client
            .post(&url)
            .header(REFERER, &referer)
            .build()
            .map_err(|_| Error::Network)?;
        let Some(permit) = self.limits.try_acquire(Route::of(&request))? else {
            return Ok(false);
        };
        self.execute(request, permit).await?;
        Ok(true)
    }

    pub async fn ack_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        days_since_discord_epoch: u32,
    ) -> Result<(), Error> {
        let referer = referer(guild_id, channel_id);
        let url =
            format!("https://discord.com/api/v9/channels/{channel_id}/messages/{message_id}/ack");
        let body = AckMessage {
            token: None,
            last_viewed: days_since_discord_epoch,
        };
        self.send(|| self.client.post(&url).header(REFERER, &referer).json(&body))
            .await?;
        Ok(())
    }

    pub async fn ack_manual(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        mention_count: u32,
    ) -> Result<(), Error> {
        let referer = referer(guild_id, channel_id);
        let url =
            format!("https://discord.com/api/v9/channels/{channel_id}/messages/{message_id}/ack");
        let body = AckManual {
            manual: true,
            mention_count,
        };
        self.send(|| self.client.post(&url).header(REFERER, &referer).json(&body))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{AckManual, forward_body};

    #[test]
    fn forward_body_matches_discord() {
        let body = forward_body(Some(200), 2002, 3003, 44);
        assert_eq!(
            serde_json::to_string(&body).unwrap(),
            r#"{"content":"","nonce":"44","tts":false,"flags":0,"mobile_network_type":"unknown","message_reference":{"guild_id":"200","channel_id":"2002","message_id":"3003","type":1}}"#
        );
    }

    #[test]
    fn ack_manual_body_matches_discord() {
        let body = AckManual {
            manual: true,
            mention_count: 3,
        };
        assert_eq!(
            serde_json::to_string(&body).unwrap(),
            r#"{"manual":true,"mention_count":3}"#
        );
    }
}
