use super::*;

fn decode_settings(value: &Value) -> Result<Vec<u8>, Error> {
    let encoded = value
        .get("settings")
        .and_then(Value::as_str)
        .ok_or(Error::Decode)?;
    STANDARD.decode(encoded).map_err(|_| Error::Decode)
}

impl Rest {
    pub async fn fetch_frecency_settings(&self) -> Result<Vec<u8>, Error> {
        let response = self
            .send(|| {
                self.client
                    .get(FRECENCY_SETTINGS)
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        decode_settings(&value)
    }

    pub async fn update_frecency_settings(&self, proto: &[u8]) -> Result<Vec<u8>, Error> {
        let body = serde_json::json!({"settings": STANDARD.encode(proto)});
        let response = self
            .send(|| {
                self.client
                    .patch(FRECENCY_SETTINGS)
                    .header(REFERER, "https://discord.com/channels/@me")
                    .json(&body)
            })
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        decode_settings(&value)
    }
}
