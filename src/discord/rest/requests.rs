use super::*;

const RELATIONSHIPS_URL: &str = "https://discord.com/api/v9/users/@me/relationships";
const FRIEND_SUGGESTIONS_URL: &str = "https://discord.com/api/v9/friend-suggestions";
const INSTALLATION_URL: &str = "https://discord.com/api/v9/apex/experiments?surface=2";
// `{"location":"user profile modal v2"}`; matches the official profile modal.
const PROFILE_CONTEXT: &str = "eyJsb2NhdGlvbiI6InVzZXIgcHJvZmlsZSBtb2RhbCB2MiJ9";

#[derive(Serialize)]
struct OpenDm {
    recipients: Vec<String>,
}

#[derive(Serialize)]
struct AddFriend<'a> {
    username: &'a str,
    discriminator: Option<String>,
}

#[derive(Serialize)]
struct EmptyBody {}

fn relationship_url(user_id: u64) -> String {
    format!("{RELATIONSHIPS_URL}/{user_id}")
}

fn friend_request(client: &Client, username: &str) -> RequestBuilder {
    client.post(RELATIONSHIPS_URL).json(&AddFriend {
        username,
        discriminator: None,
    })
}

fn relationship_put(client: &Client, user_id: u64, referer: &str) -> RequestBuilder {
    client
        .put(relationship_url(user_id))
        .header("origin", "https://discord.com")
        .header(REFERER, referer)
        .header("x-context-properties", PROFILE_CONTEXT)
        .json(&EmptyBody {})
}

fn relationship_delete(client: &Client, user_id: u64) -> RequestBuilder {
    client.delete(relationship_url(user_id))
}

#[derive(Serialize)]
struct AcceptMessageRequest {
    consent_status: u8,
}

impl Rest {
    pub async fn send_friend_request(&self, username: &str) -> Result<(), Error> {
        self.send(|| friend_request(&self.client, username)).await?;
        Ok(())
    }

    pub async fn put_relationship(
        &self,
        user_id: u64,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
    ) -> Result<(), Error> {
        let referer = channel_id.map_or_else(
            || "https://discord.com/channels/@me".to_owned(),
            |channel_id| referer(guild_id, channel_id),
        );
        if self
            .fetch_installation(INSTALLATION_URL, &referer)
            .await
            .is_err()
        {
            log::warn!("Could not fetch Discord installation metadata");
        }
        self.send(|| relationship_put(&self.client, user_id, &referer))
            .await?;
        Ok(())
    }

    pub async fn delete_relationship(&self, user_id: u64) -> Result<(), Error> {
        self.send(|| relationship_delete(&self.client, user_id))
            .await?;
        Ok(())
    }

    pub async fn fetch_friend_suggestions(&self) -> Result<Vec<model::FriendSuggestion>, Error> {
        let response = self
            .send(|| {
                self.client
                    .get(FRIEND_SUGGESTIONS_URL)
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        Ok(model::parse_entries(value))
    }

    pub async fn open_dm(&self, user_id: u64) -> Result<model::Channel, Error> {
        let body = OpenDm {
            recipients: vec![user_id.to_string()],
        };
        let response = self
            .send(|| {
                self.client
                    .post("https://discord.com/api/v9/users/@me/channels")
                    .json(&body)
            })
            .await?;
        response
            .json::<model::Channel>()
            .await
            .map_err(|_| Error::Decode)
    }

    pub async fn accept_message_request(&self, channel_id: u64) -> Result<model::Channel, Error> {
        let referer = referer(None, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/recipients/@me");
        let response = self
            .send(|| {
                self.client
                    .put(&url)
                    .header(REFERER, &referer)
                    .json(&AcceptMessageRequest { consent_status: 2 })
            })
            .await?;
        response
            .json::<model::Channel>()
            .await
            .map_err(|_| Error::Decode)
    }

    pub async fn reject_message_request(&self, channel_id: u64) -> Result<(), Error> {
        let referer = referer(None, channel_id);
        let url = format!("https://discord.com/api/v9/channels/{channel_id}/recipients/@me");
        self.send(|| self.client.delete(&url).header(REFERER, &referer))
            .await?;
        Ok(())
    }

    pub async fn message_request_previews(
        &self,
        channel_ids: &[u64],
    ) -> Result<Vec<model::MessageRequestPreview>, Error> {
        let mut previews = Vec::new();
        for chunk in channel_ids.chunks(MESSAGE_REQUEST_PREVIEW_LIMIT) {
            let query: Vec<_> = chunk
                .iter()
                .map(|channel_id| ("channel_ids", channel_id.to_string()))
                .collect();
            let response = self
                .send(|| {
                    self.client
                        .get(
                            "https://discord.com/api/v9/users/@me/message-requests/supplemental-data",
                        )
                        .header(REFERER, "https://discord.com/channels/@me")
                        .query(&query)
                })
                .await?;
            let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
            previews.extend(model::parse_entries(value));
        }
        Ok(previews)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;

    #[test]
    fn relationship_requests_use_the_friend_routes() {
        let client = Client::new();

        let request = friend_request(&client, "a.b_c1").build().unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/users/@me/relationships"
        );
        assert_eq!(
            request.body().and_then(reqwest::Body::as_bytes),
            Some(br#"{"username":"a.b_c1","discriminator":null}"#.as_slice())
        );

        let referer = super::referer(Some(7), 8);
        let request = relationship_put(&client, 42, &referer).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::PUT);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/users/@me/relationships/42"
        );
        assert_eq!(
            request.body().and_then(reqwest::Body::as_bytes),
            Some(b"{}".as_slice())
        );
        assert_eq!(
            request.headers()["x-context-properties"],
            "eyJsb2NhdGlvbiI6InVzZXIgcHJvZmlsZSBtb2RhbCB2MiJ9"
        );
        assert_eq!(request.headers()["origin"], "https://discord.com");
        assert_eq!(
            request.headers()[REFERER],
            "https://discord.com/channels/7/8"
        );
        assert_eq!(request.headers()["content-type"], "application/json");
        let request = relationship_put(&client, 42, &super::referer(None, 8))
            .build()
            .unwrap();
        assert_eq!(
            request.headers()[REFERER],
            "https://discord.com/channels/@me/8"
        );
        let request = relationship_put(&client, 42, "https://discord.com/channels/@me")
            .build()
            .unwrap();
        assert_eq!(
            request.headers()[REFERER],
            "https://discord.com/channels/@me"
        );

        let request = relationship_delete(&client, 42).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::DELETE);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/users/@me/relationships/42"
        );
        assert!(request.body().is_none());
    }
}
