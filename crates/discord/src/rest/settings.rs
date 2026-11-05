use super::*;

fn decode_settings(value: &Value) -> Result<Vec<u8>, Error> {
    let encoded = value
        .get("settings")
        .and_then(Value::as_str)
        .ok_or(Error::Decode)?;
    STANDARD.decode(encoded).map_err(|_| Error::Decode)
}

fn status_response(value: &Value) -> Result<Vec<u8>, Error> {
    if value.get("out_of_date").and_then(Value::as_bool) == Some(true) {
        return Err(Error::SettingsOutdated);
    }
    decode_settings(value)
}

impl Rest {
    pub async fn fetch_preloaded_settings(&self) -> Result<Vec<u8>, Error> {
        let response = self
            .send(|| {
                self.client
                    .get(PRELOADED_SETTINGS)
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        decode_settings(&value)
    }

    pub async fn update_status_settings(
        &self,
        proto: &[u8],
        version: u64,
    ) -> Result<Vec<u8>, Error> {
        self.update_preloaded_settings(proto, version).await
    }

    /// CAS PATCH of PreloadedUserSettings; the server rejects a stale `required_data_version`.
    pub async fn update_preloaded_settings(
        &self,
        proto: &[u8],
        version: u64,
    ) -> Result<Vec<u8>, Error> {
        let body = serde_json::json!({
            "settings": STANDARD.encode(proto), "required_data_version": version
        });
        let response = self
            .send(|| {
                self.client
                    .patch(PRELOADED_SETTINGS)
                    .header(REFERER, "https://discord.com/channels/@me")
                    .json(&body)
            })
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        status_response(&value)
    }

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

#[cfg(test)]
mod tests {
    use super::{Error, STANDARD, status_response};
    use base64::Engine;
    use serde_json::json;

    #[test]
    fn a_discarded_compare_and_swap_is_not_reported_as_a_successful_patch() {
        assert!(matches!(
            status_response(&json!({"settings": STANDARD.encode([8, 1]), "out_of_date": true})),
            Err(Error::SettingsOutdated)
        ));
        assert_eq!(
            status_response(&json!({"settings": STANDARD.encode([8, 1])})).unwrap(),
            [8, 1]
        );
        assert!(matches!(status_response(&json!({})), Err(Error::Decode)));
    }
}
