use super::*;

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
            .send(|| self.client.post(&url).header(REFERER, &referer).json(&body))
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
    use super::AckManual;

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
