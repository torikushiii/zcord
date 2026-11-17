use super::*;

const RELATIONSHIPS_URL: &str = "https://discord.com/api/v9/users/@me/relationships";
const FRIEND_SUGGESTIONS_URL: &str = "https://discord.com/api/v9/friend-suggestions";
const INSTALLATION_URL: &str = "https://discord.com/api/v9/apex/experiments?surface=2";
const FRIENDS_REFERER: &str = "https://discord.com/channels/@me";
const EMPTY_CONTEXT: &str = "e30=";
const ADD_FRIEND_CONTEXT: &str = "eyJsb2NhdGlvbiI6IkFkZCBGcmllbmQifQ==";
pub(super) const CONTEXT_MENU_CONTEXT: &str = "eyJsb2NhdGlvbiI6IkNvbnRleHRNZW51In0=";
const PROFILE_MODAL_CONTEXT: &str = "eyJsb2NhdGlvbiI6InVzZXIgcHJvZmlsZSBtb2RhbCB2MiJ9";

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
struct PutRelationship {
    #[serde(skip_serializing_if = "Option::is_none")]
    confirm_stranger_request: Option<bool>,
}

pub(super) fn relationship_context(location: model::RelationshipLocation) -> &'static str {
    match location {
        model::RelationshipLocation::ContextMenu => CONTEXT_MENU_CONTEXT,
        model::RelationshipLocation::ProfileModal => PROFILE_MODAL_CONTEXT,
    }
}

fn relationship_url(user_id: u64) -> String {
    format!("{RELATIONSHIPS_URL}/{user_id}")
}

fn friend_request(client: &Client, username: &str) -> RequestBuilder {
    client
        .post(RELATIONSHIPS_URL)
        .header(REFERER, FRIENDS_REFERER)
        .header("x-context-properties", ADD_FRIEND_CONTEXT)
        .json(&AddFriend {
            username,
            discriminator: None,
        })
}

fn relationship_put(
    client: &Client,
    user_id: u64,
    referer: &str,
    location: model::RelationshipLocation,
    confirm_stranger_request: Option<bool>,
) -> RequestBuilder {
    client
        .put(relationship_url(user_id))
        .header("origin", "https://discord.com")
        .header(REFERER, referer)
        .header("x-context-properties", relationship_context(location))
        .json(&PutRelationship {
            confirm_stranger_request,
        })
}

fn relationship_delete(
    client: &Client,
    user_id: u64,
    location: model::RelationshipLocation,
) -> RequestBuilder {
    client
        .delete(relationship_url(user_id))
        .header("x-context-properties", relationship_context(location))
}

fn open_dm(client: &Client, user_id: u64) -> RequestBuilder {
    client
        .post("https://discord.com/api/v9/users/@me/channels")
        .header("x-context-properties", EMPTY_CONTEXT)
        .json(&OpenDm {
            recipients: vec![user_id.to_string()],
        })
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

    async fn put_relationship(
        &self,
        user_id: u64,
        location: model::RelationshipLocation,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
        confirm_stranger_request: Option<bool>,
    ) -> Result<(), Error> {
        let referer = channel_id.map_or_else(
            || FRIENDS_REFERER.to_owned(),
            |channel_id| referer(guild_id, channel_id),
        );
        if self
            .fetch_installation(INSTALLATION_URL, &referer)
            .await
            .is_err()
        {
            log::warn!("Could not fetch Discord installation metadata");
        }
        self.send(|| {
            relationship_put(
                &self.client,
                user_id,
                &referer,
                location,
                confirm_stranger_request,
            )
        })
        .await?;
        Ok(())
    }

    pub async fn add_friend(
        &self,
        user_id: u64,
        location: model::RelationshipLocation,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
    ) -> Result<(), Error> {
        self.put_relationship(user_id, location, guild_id, channel_id, None)
            .await
    }

    pub async fn accept_friend_request(
        &self,
        user_id: u64,
        location: model::RelationshipLocation,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
    ) -> Result<(), Error> {
        let confirm_stranger_request =
            (location == model::RelationshipLocation::ProfileModal).then_some(false);
        self.put_relationship(
            user_id,
            location,
            guild_id,
            channel_id,
            confirm_stranger_request,
        )
        .await
    }

    pub async fn delete_relationship(
        &self,
        user_id: u64,
        location: model::RelationshipLocation,
    ) -> Result<(), Error> {
        self.send(|| relationship_delete(&self.client, user_id, location))
            .await?;
        Ok(())
    }

    pub async fn fetch_friend_suggestions(&self) -> Result<Vec<model::FriendSuggestion>, Error> {
        let response = self
            .send(|| {
                self.client
                    .get(FRIEND_SUGGESTIONS_URL)
                    .header(REFERER, FRIENDS_REFERER)
            })
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        Ok(model::parse_entries(value))
    }

    pub async fn open_dm(&self, user_id: u64) -> Result<model::Channel, Error> {
        let response = self.send(|| open_dm(&self.client, user_id)).await?;
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
                        .header(REFERER, "https://discord.com/message-requests")
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

    fn context(request: &reqwest::Request) -> Option<&str> {
        request
            .headers()
            .get("x-context-properties")
            .map(|value| value.to_str().unwrap())
    }

    #[test]
    fn relationship_requests_use_the_friend_routes() {
        let client = Client::new();

        let request = friend_request(&client, "A.b_c1").build().unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/users/@me/relationships"
        );
        assert_eq!(
            request.body().and_then(reqwest::Body::as_bytes),
            Some(br#"{"username":"A.b_c1","discriminator":null}"#.as_slice())
        );
        assert_eq!(context(&request), Some(ADD_FRIEND_CONTEXT));
        assert_eq!(request.headers()[REFERER], FRIENDS_REFERER);

        let referer = super::referer(Some(7), 8);
        let request = relationship_put(
            &client,
            42,
            &referer,
            model::RelationshipLocation::ProfileModal,
            None,
        )
        .build()
        .unwrap();
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
            context(&request),
            Some("eyJsb2NhdGlvbiI6InVzZXIgcHJvZmlsZSBtb2RhbCB2MiJ9")
        );
        assert_eq!(request.headers()["origin"], "https://discord.com");
        assert_eq!(
            request.headers()[REFERER],
            "https://discord.com/channels/7/8"
        );
        assert_eq!(request.headers()["content-type"], "application/json");

        let request = relationship_put(
            &client,
            42,
            FRIENDS_REFERER,
            model::RelationshipLocation::ContextMenu,
            Some(false),
        )
        .build()
        .unwrap();
        assert_eq!(
            request.body().and_then(reqwest::Body::as_bytes),
            Some(br#"{"confirm_stranger_request":false}"#.as_slice())
        );
        assert_eq!(context(&request), Some(CONTEXT_MENU_CONTEXT));
        assert_eq!(request.headers()[REFERER], FRIENDS_REFERER);

        let request = relationship_delete(&client, 42, model::RelationshipLocation::ContextMenu)
            .build()
            .unwrap();
        assert_eq!(request.method(), reqwest::Method::DELETE);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/users/@me/relationships/42"
        );
        assert!(request.body().is_none());
        assert_eq!(context(&request), Some(CONTEXT_MENU_CONTEXT));
    }

    #[test]
    fn open_dm_sends_an_empty_context() {
        let request = open_dm(&Client::new(), 42).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/users/@me/channels"
        );
        assert_eq!(
            request.body().and_then(reqwest::Body::as_bytes),
            Some(br#"{"recipients":["42"]}"#.as_slice())
        );
        assert_eq!(context(&request), Some("e30="));
    }
}
