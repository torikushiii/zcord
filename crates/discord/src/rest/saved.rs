use super::*;
use serde_json::json;

const SAVED: &str = "https://discord.com/api/v9/users/@me/saved-messages";

fn saved_url(channel_id: u64, message_id: u64) -> String {
    format!("{SAVED}/{channel_id}/{message_id}")
}

fn saved_messages_request(client: &Client) -> RequestBuilder {
    client.get(format!("{SAVED}?bookmark_ids=true"))
}

fn bookmarks_request(client: &Client, before: Option<&str>) -> RequestBuilder {
    let request = client
        .get(format!("{SAVED}/bookmarks"))
        .query(&[("limit", "25")]);
    match before {
        Some(before) => request.query(&[("before", before)]),
        None => request,
    }
}

fn save_request(
    client: &Client,
    channel_id: u64,
    message_id: u64,
    due_at: Option<&str>,
) -> RequestBuilder {
    let body = match due_at {
        Some(due_at) => json!({"due_at": due_at, "source": "context_menu"}),
        None => json!({"source": "context_menu"}),
    };
    client.put(saved_url(channel_id, message_id)).json(&body)
}

fn unsave_request(client: &Client, channel_id: u64, message_id: u64) -> RequestBuilder {
    client.delete(saved_url(channel_id, message_id))
}

const UNKNOWN_SAVED_MESSAGES: u64 = 10133;

fn never_saved(error: &Error) -> bool {
    matches!(
        error,
        Error::Rejected {
            status: 404,
            code: UNKNOWN_SAVED_MESSAGES,
            ..
        }
    )
}

impl Rest {
    pub async fn saved_messages(&self) -> Result<model::SavedMessages, Error> {
        match self.send(|| saved_messages_request(&self.client)).await {
            Ok(response) => response.json().await.map_err(|_| Error::Decode),
            Err(error) if never_saved(&error) => Ok(model::SavedMessages::default()),
            Err(error) => Err(error),
        }
    }

    pub async fn bookmarks(&self, before: Option<&str>) -> Result<model::BookmarkPage, Error> {
        match self.send(|| bookmarks_request(&self.client, before)).await {
            Ok(response) => response.json().await.map_err(|_| Error::Decode),
            Err(error) if never_saved(&error) => Ok(model::BookmarkPage::default()),
            Err(error) => Err(error),
        }
    }

    pub async fn save_message(
        &self,
        channel_id: u64,
        message_id: u64,
        due_at: Option<&str>,
    ) -> Result<model::SavedMessage, Error> {
        let response = self
            .send(|| save_request(&self.client, channel_id, message_id, due_at))
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn unsave_message(&self, channel_id: u64, message_id: u64) -> Result<(), Error> {
        self.send(|| unsave_request(&self.client, channel_id, message_id))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(request: &Request) -> Value {
        serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap()
    }

    #[test]
    fn saved_message_requests_use_the_saved_messages_routes() {
        let client = Client::new();
        let request = saved_messages_request(&client).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::GET);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/users/@me/saved-messages?bookmark_ids=true"
        );

        let request = bookmarks_request(&client, None).build().unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/users/@me/saved-messages/bookmarks?limit=25"
        );
        let request = bookmarks_request(&client, Some("2026-11-06T20:11:29.594541+00:00"))
            .build()
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/users/@me/saved-messages/bookmarks?limit=25&before=2026-11-06T20%3A11%3A29.594541%2B00%3A00"
        );

        let request = save_request(&client, 20, 30, None).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::PUT);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/users/@me/saved-messages/20/30"
        );
        assert_eq!(body(&request), json!({"source": "context_menu"}));

        let request = save_request(&client, 20, 30, Some("2026-11-07T11:22:20.836Z"))
            .build()
            .unwrap();
        assert_eq!(
            body(&request),
            json!({"due_at": "2026-11-07T11:22:20.836Z", "source": "context_menu"})
        );

        let request = unsave_request(&client, 20, 30).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::DELETE);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/users/@me/saved-messages/20/30"
        );
    }

    #[test]
    fn only_unknown_saved_messages_counts_as_never_saved() {
        let rejected = |status, code| Error::Rejected {
            status,
            code,
            message: "Unknown Saved Messages".into(),
        };
        assert!(never_saved(&rejected(404, 10133)));
        assert!(!never_saved(&rejected(404, 10008)));
        assert!(!never_saved(&rejected(403, 10133)));
        assert!(!never_saved(&Error::Http(404)));
    }
}
