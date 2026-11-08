use super::*;
use crate::{ForumQuery, ForumSort, SearchOutcome, SearchQuery, SearchScope};
use futures::Stream;
use reqwest::{
    Body,
    header::{CONTENT_LENGTH, CONTENT_TYPE, RETRY_AFTER},
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
pub(super) fn forum_search_query(query: &ForumQuery) -> Vec<(&'static str, String)> {
    let sort_by = match query.sort {
        ForumSort::Activity => "last_message_time",
        ForumSort::Created => "creation_time",
    };
    let mut params = vec![
        ("sort_by", sort_by.to_owned()),
        ("sort_order", "desc".to_owned()),
        ("limit", "25".to_owned()),
    ];
    params.extend(query.tags.iter().map(|tag| ("tag", tag.to_string())));
    let tag_setting = if query.match_all {
        "match_all"
    } else {
        "match_some"
    };
    params.push(("tag_setting", tag_setting.to_owned()));
    params.push(("offset", query.offset.to_string()));
    params
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    sticker_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_inventory_entry: Option<ContentInventoryEntryBody<'a>>,
}

#[derive(Serialize)]
struct ContentInventoryEntryBody<'a> {
    unverified_content: &'a serde_json::Value,
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
        sticker_ids: message
            .sticker_ids
            .iter()
            .map(|id| id.to_string())
            .collect(),
        content_inventory_entry: message.content_inventory_entry.as_deref().map(|entry| {
            ContentInventoryEntryBody {
                unverified_content: entry,
            }
        }),
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

fn reactions_url(channel_id: u64, message_id: u64, emoji: &str) -> String {
    format!(
        "https://discord.com/api/v9/channels/{channel_id}/messages/{message_id}/reactions/{}",
        percent_encode(emoji)
    )
}

pub(super) fn reaction_url(channel_id: u64, message_id: u64, emoji: &str) -> String {
    format!("{}/@me", reactions_url(channel_id, message_id, emoji))
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
        query: &ForumQuery,
    ) -> Result<model::ForumSearch, Error> {
        let referer = referer(guild_id, forum_id);
        let url = format!("https://discord.com/api/v9/channels/{forum_id}/threads/search");
        let query = forum_search_query(query);
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

    pub async fn search_messages(
        &self,
        scope: SearchScope,
        query: &SearchQuery,
    ) -> Result<SearchOutcome, Error> {
        let (url, referer) = match scope {
            SearchScope::Guild(guild_id) => (
                format!("https://discord.com/api/v9/guilds/{guild_id}/messages/search"),
                format!("https://discord.com/channels/{guild_id}"),
            ),
            SearchScope::Channel(channel_id) => (
                format!("https://discord.com/api/v9/channels/{channel_id}/messages/search"),
                "https://discord.com/channels/@me".to_owned(),
            ),
            SearchScope::Dms => (
                "https://discord.com/api/v9/users/@me/messages/search/tabs".to_owned(),
                "https://discord.com/channels/@me".to_owned(),
            ),
        };
        let params = crate::search::search_params(scope, query);
        let body = crate::search::dm_search_body(query);
        let response = self
            .send(|| {
                let request = match scope {
                    SearchScope::Dms => self.client.post(&url).json(&body),
                    SearchScope::Guild(_) | SearchScope::Channel(_) => {
                        self.client.get(&url).query(&params)
                    }
                };
                request.header(REFERER, &referer)
            })
            .await;
        parse_search_response(response, scope).await
    }

    pub(crate) async fn fetch_messages_anchor(
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

    pub(crate) async fn add_reaction(
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

    pub(crate) async fn remove_reaction(
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

    pub(crate) async fn reaction_users(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        emoji: &str,
        page: model::ReactorPage,
    ) -> Result<Vec<model::User>, Error> {
        let referer = referer(guild_id, channel_id);
        let url = reactions_url(channel_id, message_id, emoji);
        let mut query = vec![
            ("limit", page.limit.to_string()),
            ("type", u8::from(page.burst).to_string()),
        ];
        query.extend(page.after.map(|after| ("after", after.to_string())));
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .header(REFERER, &referer)
                    .query(&query)
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
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
            sticker_ids: Vec::new(),
            content_inventory_entry: None,
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

async fn parse_search_response(
    response: Result<Response, Error>,
    scope: SearchScope,
) -> Result<SearchOutcome, Error> {
    let response = match response {
        Ok(response) => response,
        Err(Error::Rejected { code, .. }) if is_search_indexing_code(code) => {
            return Ok(SearchOutcome::Indexing {
                retry_after: Duration::from_secs(2),
            });
        }
        Err(error) => return Err(error),
    };
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite());
    let value = match response.json::<Value>().await {
        Ok(value) => value,
        Err(_) if status == 202 => Value::Null,
        Err(_) => return Err(Error::Decode),
    };
    match (parse_search_reply(status, value, scope)?, retry_after) {
        (SearchOutcome::Indexing { .. }, Some(seconds)) => Ok(SearchOutcome::Indexing {
            retry_after: Duration::from_secs_f64(seconds.clamp(1.0, 30.0)),
        }),
        (outcome, _) => Ok(outcome),
    }
}

fn parse_search_reply(
    status: u16,
    mut value: Value,
    scope: SearchScope,
) -> Result<SearchOutcome, Error> {
    if status == 202
        || value
            .get("code")
            .and_then(Value::as_u64)
            .is_some_and(is_search_indexing_code)
    {
        let retry_after = value
            .get("retry_after")
            .and_then(Value::as_f64)
            .filter(|seconds| seconds.is_finite())
            .unwrap_or(2.0)
            .clamp(1.0, 30.0);
        return Ok(SearchOutcome::Indexing {
            retry_after: Duration::from_secs_f64(retry_after),
        });
    }
    if scope == SearchScope::Dms {
        value = value
            .get_mut("tabs")
            .and_then(|tabs| tabs.get_mut("messages"))
            .filter(|messages| messages.is_object())
            .ok_or(Error::Decode)?
            .take();
    }
    serde_json::from_value(value)
        .map(SearchOutcome::Page)
        .map_err(|_| Error::Decode)
}

fn is_search_indexing_code(code: u64) -> bool {
    (110_000..=119_999).contains(&code)
}

#[cfg(test)]
mod tests {
    use super::{AckManual, forward_body, message_body, parse_search_reply, parse_search_response};
    use crate::{Error, OutgoingMessage, SearchOutcome, SearchScope};
    use std::time::Duration;

    #[test]
    fn message_body_serializes_sticker_ids_only_when_non_empty() {
        let message = OutgoingMessage {
            content: String::new(),
            nonce: 7,
            reply: None,
            attachments: Vec::new(),
            sticker_ids: Vec::new(),
            content_inventory_entry: None,
        };
        let body = serde_json::to_string(&message_body(None, 20, &message)).unwrap();
        assert_eq!(
            body,
            r#"{"mobile_network_type":"unknown","content":"","nonce":"7","tts":false,"flags":0}"#
        );
        let message = OutgoingMessage {
            content: String::new(),
            nonce: 7,
            reply: None,
            attachments: Vec::new(),
            sticker_ids: vec![9001, 9002],
            content_inventory_entry: None,
        };
        let body = serde_json::to_string(&message_body(None, 20, &message)).unwrap();
        assert_eq!(
            body,
            r#"{"mobile_network_type":"unknown","content":"","nonce":"7","tts":false,"flags":0,"sticker_ids":["9001","9002"]}"#
        );
    }

    #[test]
    fn message_body_wraps_the_content_inventory_entry_verbatim() {
        let entry = serde_json::json!({
            "id": "1191169321074688097", "author_id": "1191169170079744061",
            "author_type": 0, "content_type": 1,
            "traits": [{"type": 3, "is_live": true}],
            "extra": {"type": "played_game_extra", "game_name": "Synthetic"},
            "participants": ["1191169170079744061"],
            "signature": "synthetic-signature"
        });
        let message = OutgoingMessage {
            content: "wave".to_owned(),
            nonce: 7,
            reply: None,
            attachments: Vec::new(),
            sticker_ids: Vec::new(),
            content_inventory_entry: Some(std::sync::Arc::new(entry)),
        };
        let body = serde_json::to_string(&message_body(None, 20, &message)).unwrap();
        assert_eq!(
            body,
            r#"{"mobile_network_type":"unknown","content":"wave","nonce":"7","tts":false,"flags":0,"content_inventory_entry":{"unverified_content":{"id":"1191169321074688097","author_id":"1191169170079744061","author_type":0,"content_type":1,"traits":[{"type":3,"is_live":true}],"extra":{"type":"played_game_extra","game_name":"Synthetic"},"participants":["1191169170079744061"],"signature":"synthetic-signature"}}}"#
        );
    }

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

    #[test]
    fn search_dms_reply_maps_only_messages_tab_and_preserves_channels() {
        let message = |id: &str, channel_id: &str| {
            serde_json::json!({
                "id": id,
                "channel_id": channel_id,
                "author": {"id": "11"},
                "timestamp": "2024-01-01T00:00:00.000Z"
            })
        };
        let value = serde_json::json!({
            "analytics_id": "search-id",
            "tabs": {
                "messages": {
                    "total_results": 42,
                    "messages": [
                        [message("1", "10"), message("9", "10")],
                        [],
                        [null, message("8", "20")],
                        [message("2", "20")]
                    ],
                    "channels": [
                        {"id": "10", "type": 1, "recipients": [{"id": "11"}]},
                        null,
                        {"id": "20", "type": 3, "name": "Group DM"}
                    ],
                    "cursor": {"type": "timestamp", "timestamp": "2"}
                },
                "media": {"total_results": 1, "messages": [[message("3", "30")]]}
            }
        });
        let outcome = parse_search_reply(200, value, SearchScope::Dms).unwrap();
        let SearchOutcome::Page(page) = outcome else {
            panic!("successful DM search must return a page");
        };
        assert_eq!(page.total, 42);
        assert_eq!(
            page.messages
                .iter()
                .map(|message| (message.id, message.channel_id))
                .collect::<Vec<_>>(),
            [(1, 10), (2, 20)]
        );
        assert_eq!(
            page.channels
                .iter()
                .map(|channel| channel.id)
                .collect::<Vec<_>>(),
            [10, 20]
        );
        assert_eq!(page.channels[0].recipients[0].id, 11);
    }

    #[test]
    fn search_dms_reply_requires_messages_tab_but_accepts_empty_results() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"messages": [], "total_results": 0}),
            serde_json::json!({"tabs": {}}),
            serde_json::json!({"tabs": {"messages": null}}),
            serde_json::json!({"tabs": {"messages": []}}),
        ] {
            assert!(matches!(
                parse_search_reply(200, value, SearchScope::Dms),
                Err(Error::Decode)
            ));
        }
        let value = serde_json::json!({
            "tabs": {"messages": {
                "total_results": 0, "messages": [], "channels": [], "cursor": {}
            }}
        });
        let outcome = parse_search_reply(200, value, SearchScope::Dms).unwrap();
        let SearchOutcome::Page(page) = outcome else {
            panic!("empty DM search must return a page");
        };
        assert_eq!(page.total, 0);
        assert!(page.messages.is_empty());
        assert!(page.channels.is_empty());
    }

    #[test]
    fn search_dms_indexing_reply_is_parsed_before_the_tabs_envelope() {
        for (status, value) in [
            (202, serde_json::Value::Null),
            (202, serde_json::json!({"retry_after": 2})),
            (200, serde_json::json!({"code": 110000, "retry_after": 2})),
        ] {
            assert!(matches!(
                parse_search_reply(status, value, SearchScope::Dms),
                Ok(SearchOutcome::Indexing { retry_after }) if retry_after == Duration::from_secs(2)
            ));
        }
    }

    #[test]
    fn search_202_reply_returns_clamped_indexing_delay() {
        let outcome = parse_search_reply(
            202,
            serde_json::json!({"code": 110000, "retry_after": 1.5}),
            SearchScope::Channel(10),
        )
        .unwrap();
        assert!(matches!(
            outcome,
            SearchOutcome::Indexing { retry_after } if retry_after == Duration::from_millis(1500)
        ));

        for (requested, expected) in [
            (-1.0, 1),
            (0.2, 1),
            (1.0, 1),
            (30.0, 30),
            (31.0, 30),
            (1e100, 30),
        ] {
            let outcome = parse_search_reply(
                202,
                serde_json::json!({"retry_after": requested}),
                SearchScope::Channel(10),
            )
            .unwrap();
            let SearchOutcome::Indexing { retry_after } = outcome else {
                panic!("202 response must be treated as indexing");
            };
            assert_eq!(retry_after, Duration::from_secs(expected));
        }
    }

    #[test]
    fn search_indexing_code_and_missing_retry_delay_are_tolerated() {
        for code in [110000, 115000, 119999] {
            let outcome = parse_search_reply(
                200,
                serde_json::json!({"code": code}),
                SearchScope::Channel(10),
            )
            .unwrap();
            assert!(matches!(
                outcome,
                SearchOutcome::Indexing { retry_after } if retry_after == Duration::from_secs(2)
            ));
            let outcome = parse_search_reply(
                200,
                serde_json::json!({"code": code, "retry_after": 4.5}),
                SearchScope::Channel(10),
            )
            .unwrap();
            assert!(matches!(
                outcome,
                SearchOutcome::Indexing { retry_after } if retry_after == Duration::from_millis(4500)
            ));
        }
        for value in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({"code": 50035, "retry_after": null}),
            serde_json::json!({"retry_after": "invalid"}),
        ] {
            let outcome = parse_search_reply(202, value, SearchScope::Channel(10)).unwrap();
            assert!(matches!(
                outcome,
                SearchOutcome::Indexing { retry_after } if retry_after == Duration::from_secs(2)
            ));
        }
        for code in [109999, 120000] {
            assert!(matches!(
                parse_search_reply(
                    200,
                    serde_json::json!({"code": code}),
                    SearchScope::Channel(10),
                ),
                Ok(SearchOutcome::Page(_))
            ));
        }
    }

    #[tokio::test]
    async fn search_rejected_indexing_codes_use_the_default_delay() {
        for code in [110000, 115000, 119999] {
            let outcome = parse_search_response(
                Err(Error::Rejected {
                    status: 400,
                    code,
                    message: "Indexing".to_owned(),
                }),
                SearchScope::Channel(10),
            )
            .await
            .unwrap();
            assert!(matches!(
                outcome,
                SearchOutcome::Indexing { retry_after } if retry_after == Duration::from_secs(2)
            ));
        }
        for code in [109999, 120000] {
            let outcome = parse_search_response(
                Err(Error::Rejected {
                    status: 403,
                    code,
                    message: "Rejected".to_owned(),
                }),
                SearchScope::Channel(10),
            )
            .await;
            assert!(matches!(
                outcome,
                Err(Error::Rejected { status: 403, code: actual, message })
                    if actual == code && message == "Rejected"
            ));
        }
        assert!(matches!(
            parse_search_response(Err(Error::Http(401)), SearchScope::Channel(10)).await,
            Err(Error::Http(401))
        ));
        assert!(matches!(
            parse_search_response(Err(Error::Network), SearchScope::Channel(10)).await,
            Err(Error::Network)
        ));
        assert!(matches!(
            parse_search_response(
                Err(Error::RateLimited {
                    retry_after: Duration::from_secs(7),
                }),
                SearchScope::Channel(10),
            )
            .await,
            Err(Error::RateLimited { retry_after }) if retry_after == Duration::from_secs(7)
        ));
    }

    #[tokio::test]
    async fn search_response_decodes_wire_bodies_and_tolerates_any_202() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;
        use tokio::time::timeout;

        enum Expected {
            Indexing(Duration),
            Page,
            Decode,
        }

        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let cases = [
            (
                202,
                r#"{"code":110000,"retry_after":1.5}"#,
                Expected::Indexing(Duration::from_millis(1500)),
            ),
            (202, "", Expected::Indexing(Duration::from_secs(2))),
            (202, "not json", Expected::Indexing(Duration::from_secs(2))),
            (
                200,
                r#"{"code":119999,"retry_after":1.5}"#,
                Expected::Indexing(Duration::from_millis(1500)),
            ),
            (
                200,
                r#"{"total_results":42,"messages":[[]]}"#,
                Expected::Page,
            ),
            (200, "not json", Expected::Decode),
        ]
        .map(|(status, body, expected)| (SearchScope::Channel(10), status, "", body, expected));
        for (scope, status, headers, body, expected) in cases.into_iter().chain([
            (
                SearchScope::Dms,
                202,
                "Retry-After: 4.5\r\n",
                "",
                Expected::Indexing(Duration::from_millis(4500)),
            ),
            (
                SearchScope::Dms,
                202,
                "Retry-After: invalid\r\n",
                r#"{"retry_after":1.5}"#,
                Expected::Indexing(Duration::from_millis(1500)),
            ),
            (
                SearchScope::Dms,
                202,
                "Retry-After: 0\r\n",
                "not json",
                Expected::Indexing(Duration::from_secs(1)),
            ),
            (
                SearchScope::Dms,
                202,
                "Retry-After: 100\r\n",
                "",
                Expected::Indexing(Duration::from_secs(30)),
            ),
            (
                SearchScope::Dms,
                200,
                "",
                r#"{"tabs":{"messages":{"total_results":42,"messages":[[]],"cursor":{}}}}"#,
                Expected::Page,
            ),
            (
                SearchScope::Dms,
                200,
                "",
                r#"{"total_results":42,"messages":[[]]}"#,
                Expected::Decode,
            ),
        ]) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/search", listener.local_addr().unwrap());
            let server = async {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut bytes = [0; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let count = stream.read(&mut bytes).await.unwrap();
                    assert!(count > 0 && request.len() < 8192);
                    request.extend_from_slice(&bytes[..count]);
                }
                stream.write_all(format!(
                    "HTTP/1.1 {status} OK\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                ).as_bytes()).await.unwrap();
                stream.shutdown().await.unwrap();
            };
            let ((), response) = timeout(Duration::from_secs(3), async {
                tokio::join!(server, client.get(&url).send())
            })
            .await
            .unwrap();
            let outcome = parse_search_response(Ok(response.unwrap()), scope).await;
            match expected {
                Expected::Indexing(delay) => assert!(matches!(
                    outcome,
                    Ok(SearchOutcome::Indexing { retry_after }) if retry_after == delay
                )),
                Expected::Decode => assert!(matches!(outcome, Err(Error::Decode))),
                Expected::Page => {
                    let Ok(SearchOutcome::Page(page)) = outcome else {
                        panic!("successful search must return a page");
                    };
                    assert_eq!(page.total, 42);
                    assert!(page.messages.is_empty());
                }
            }
        }
    }
}
