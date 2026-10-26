use super::*;

#[derive(Serialize)]
struct OpenDm {
    recipients: Vec<String>,
}

#[derive(Serialize)]
struct AcceptMessageRequest {
    consent_status: u8,
}

impl Rest {
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
