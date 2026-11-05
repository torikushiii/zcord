use super::*;
use crate::PinPage;
use serde::Deserialize;

fn pins_url(channel_id: u64, before: Option<&str>) -> String {
    let mut url =
        format!("https://discord.com/api/v9/channels/{channel_id}/messages/pins?limit=25");
    if let Some(before) = before {
        url.push_str("&before=");
        url.push_str(&percent_encode(before));
    }
    url
}

fn pin_url(channel_id: u64, message_id: u64) -> String {
    format!("https://discord.com/api/v9/channels/{channel_id}/messages/pins/{message_id}")
}

fn pins_request(client: &Client, channel_id: u64, before: Option<&str>) -> RequestBuilder {
    client.get(pins_url(channel_id, before))
}

fn pin_request(client: &Client, channel_id: u64, message_id: u64) -> RequestBuilder {
    client.put(pin_url(channel_id, message_id))
}

fn unpin_request(client: &Client, channel_id: u64, message_id: u64) -> RequestBuilder {
    client.delete(pin_url(channel_id, message_id))
}

fn ack_pins_request(client: &Client, channel_id: u64) -> RequestBuilder {
    client.post(format!(
        "https://discord.com/api/v9/channels/{channel_id}/pins/ack"
    ))
}

#[derive(Deserialize)]
struct PinsReply {
    #[serde(deserialize_with = "model::entries")]
    items: Vec<model::Pin>,
    has_more: bool,
}

fn parse_pins(value: Value, channel_id: u64) -> Result<PinPage, Error> {
    let reply: PinsReply = serde_json::from_value(value).map_err(|_| Error::Decode)?;
    let mut items = reply.items;
    items.retain(|pin| pin.message.channel_id == channel_id);
    Ok(PinPage {
        items,
        has_more: reply.has_more,
    })
}

impl Rest {
    pub async fn pinned_messages(
        &self,
        channel_id: u64,
        before: Option<&str>,
    ) -> Result<PinPage, Error> {
        let response = self
            .send(|| pins_request(&self.client, channel_id, before))
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        parse_pins(value, channel_id)
    }

    pub async fn pin_message(&self, channel_id: u64, message_id: u64) -> Result<(), Error> {
        self.send(|| pin_request(&self.client, channel_id, message_id))
            .await?;
        Ok(())
    }

    pub async fn unpin_message(&self, channel_id: u64, message_id: u64) -> Result<(), Error> {
        self.send(|| unpin_request(&self.client, channel_id, message_id))
            .await?;
        Ok(())
    }

    pub async fn ack_pins(&self, channel_id: u64) -> Result<(), Error> {
        self.send(|| ack_pins_request(&self.client, channel_id))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;

    #[test]
    fn pin_requests_use_the_message_pin_routes() {
        let client = Client::new();
        let request = pins_request(&client, 20, None).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::GET);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/20/messages/pins?limit=25"
        );

        let request = pins_request(&client, 20, Some("2026-09-10T11:00:00.123456789Z"))
            .build()
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/20/messages/pins?limit=25&before=2026-09-10T11%3A00%3A00.123456789Z"
        );

        let request = pin_request(&client, 20, 30).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::PUT);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/20/messages/pins/30"
        );
        assert!(request.body().is_none());

        let request = unpin_request(&client, 20, 30).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::DELETE);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/20/messages/pins/30"
        );
        assert!(request.body().is_none());

        let request = ack_pins_request(&client, 20).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/20/pins/ack"
        );
        assert!(request.body().is_none());
    }

    #[test]
    fn pin_reply_keeps_its_order_and_flags_more() {
        let page = parse_pins(
            serde_json::json!({
                "items": [
                    {"pinned_at": "2026-09-10T11:00:00Z",
                     "message": {"id": "2", "channel_id": "20", "author": {"id": "7", "username": "a"},
                                 "timestamp": "2026-09-10T10:00:00Z", "content": "older"}},
                    {"pinned_at": "2026-09-10T12:00:00Z",
                     "message": {"id": "1", "channel_id": "20", "author": {"id": "8", "username": "b"},
                                 "timestamp": "2026-09-10T09:00:00Z", "content": "newer"}}
                ],
                "has_more": true
            }),
            20,
        )
        .unwrap();
        assert!(page.has_more);
        assert_eq!(
            page.items
                .iter()
                .map(|pin| pin.message.id)
                .collect::<Vec<_>>(),
            vec![2, 1]
        );
        assert!(matches!(
            parse_pins(serde_json::json!({}), 20),
            Err(Error::Decode)
        ));
    }
}
