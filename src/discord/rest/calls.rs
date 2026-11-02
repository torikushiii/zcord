use super::*;

fn ring(client: &Client, channel_id: u64) -> RequestBuilder {
    client
        .post(format!(
            "https://discord.com/api/v9/channels/{channel_id}/call/ring"
        ))
        .json(&serde_json::json!({"recipients": null}))
}

fn stop_ringing(client: &Client, channel_id: u64, user_id: u64) -> RequestBuilder {
    client
        .post(format!(
            "https://discord.com/api/v9/channels/{channel_id}/call/stop-ringing"
        ))
        .json(&serde_json::json!({"recipients": [user_id.to_string()]}))
}

impl Rest {
    pub async fn ring_call(&self, channel_id: u64) -> Result<(), Error> {
        self.send(|| ring(&self.client, channel_id)).await?;
        Ok(())
    }

    pub async fn stop_ringing_call(&self, channel_id: u64, user_id: u64) -> Result<(), Error> {
        self.send(|| stop_ringing(&self.client, channel_id, user_id))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_requests_use_channel_routes_and_explicit_recipients() {
        let client = Client::new();
        for (request, path, body) in [
            (
                ring(&client, 42),
                "ring",
                serde_json::json!({"recipients": null}),
            ),
            (
                stop_ringing(&client, 42, 7),
                "stop-ringing",
                serde_json::json!({"recipients": ["7"]}),
            ),
        ] {
            let request = request.build().unwrap();
            assert_eq!(request.method(), reqwest::Method::POST);
            assert_eq!(
                request.url().as_str(),
                format!("https://discord.com/api/v9/channels/42/call/{path}")
            );
            assert_eq!(
                serde_json::from_slice::<Value>(request.body().unwrap().as_bytes().unwrap())
                    .unwrap(),
                body
            );
        }
    }
}
