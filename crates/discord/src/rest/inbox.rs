use super::*;
use serde_json::{Map, json};

const API: &str = "https://discord.com/api/v9/users/@me";

/// https://docs.discord.food/resources/message#get-recent-mentions
fn mentions_request(
    client: &Client,
    before: Option<u64>,
    roles: bool,
    everyone: bool,
) -> RequestBuilder {
    let mut query = vec![
        ("limit", "25".to_owned()),
        ("roles", roles.to_string()),
        ("everyone", everyone.to_string()),
    ];
    query.extend(before.map(|before| ("before", before.to_string())));
    client.get(format!("{API}/mentions")).query(&query)
}

fn scheduled_url(scheduled_id: u64) -> String {
    format!("{API}/scheduled-messages/{scheduled_id}")
}

fn update_scheduled_request(
    client: &Client,
    scheduled_id: u64,
    send_at: Option<&str>,
    content: Option<(&str, u64)>,
) -> RequestBuilder {
    let mut body = Map::new();
    if let Some(send_at) = send_at {
        body.insert("scheduled_timestamp".into(), json!(send_at));
    }
    if let Some((content, flags)) = content {
        body.insert("content".into(), json!(content));
        body.insert("flags".into(), json!(flags));
    }
    client.patch(scheduled_url(scheduled_id)).json(&body)
}

impl Rest {
    pub async fn mentions(
        &self,
        before: Option<u64>,
        roles: bool,
        everyone: bool,
    ) -> Result<Vec<model::Message>, Error> {
        let response = self
            .send(|| mentions_request(&self.client, before, roles, everyone))
            .await?;
        let value: Value = response.json().await.map_err(|_| Error::Decode)?;
        Ok(model::parse_entries(value))
    }

    pub async fn delete_mention(&self, message_id: u64) -> Result<(), Error> {
        self.send(|| self.client.delete(format!("{API}/mentions/{message_id}")))
            .await?;
        Ok(())
    }

    pub async fn scheduled_messages(&self) -> Result<Vec<model::ScheduledMessage>, Error> {
        let response = self
            .send(|| self.client.get(format!("{API}/scheduled-messages")))
            .await?;
        let value: Value = response.json().await.map_err(|_| Error::Decode)?;
        Ok(model::parse_entries(value))
    }

    pub async fn cancel_scheduled(&self, scheduled_id: u64) -> Result<(), Error> {
        self.send(|| self.client.delete(scheduled_url(scheduled_id)))
            .await?;
        Ok(())
    }

    pub async fn send_scheduled_now(&self, scheduled_id: u64) -> Result<(), Error> {
        self.send(|| {
            self.client
                .post(format!("{}/send", scheduled_url(scheduled_id)))
        })
        .await?;
        Ok(())
    }

    pub async fn update_scheduled(
        &self,
        scheduled_id: u64,
        send_at: Option<&str>,
        content: Option<(&str, u64)>,
    ) -> Result<model::ScheduledMessage, Error> {
        let response = self
            .send(|| update_scheduled_request(&self.client, scheduled_id, send_at, content))
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(request: &Request) -> Value {
        serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap()
    }

    #[test]
    fn inbox_requests_use_the_mentions_and_scheduled_routes() {
        let client = Client::new();
        let request = mentions_request(&client, None, true, false)
            .build()
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/users/@me/mentions?limit=25&roles=true&everyone=false"
        );
        let request = mentions_request(&client, Some(9), false, true)
            .build()
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/users/@me/mentions?limit=25&roles=false&everyone=true&before=9"
        );

        let request = update_scheduled_request(&client, 5, Some("2026-11-07T13:00:00.000Z"), None)
            .build()
            .unwrap();
        assert_eq!(request.method(), reqwest::Method::PATCH);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/users/@me/scheduled-messages/5"
        );
        assert_eq!(
            body(&request),
            json!({"scheduled_timestamp": "2026-11-07T13:00:00.000Z"})
        );
        let request = update_scheduled_request(&client, 5, None, Some(("hi", 4096)))
            .build()
            .unwrap();
        assert_eq!(body(&request), json!({"content": "hi", "flags": 4096}));
    }
}
