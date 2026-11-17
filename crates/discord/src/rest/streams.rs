use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Preview {
    url: String,
}

pub(super) fn stream_preview_url(stream_key: &str) -> String {
    format!("https://discord.com/api/v9/streams/{stream_key}/preview")
}

fn parse_preview(bytes: &[u8]) -> Result<String, Error> {
    serde_json::from_slice::<Preview>(bytes)
        .map(|preview| preview.url)
        .map_err(|_| Error::Decode)
}

fn upload_preview(client: &Client, stream_key: &str, thumbnail: &str) -> RequestBuilder {
    client
        .post(stream_preview_url(stream_key))
        .json(&serde_json::json!({"thumbnail": thumbnail}))
}

fn fetch_preview(client: &Client, stream_key: &str, version: i64) -> RequestBuilder {
    client.get(format!(
        "{}?version={version}",
        stream_preview_url(stream_key)
    ))
}

impl Rest {
    pub async fn upload_stream_preview(
        &self,
        stream_key: &str,
        thumbnail: &str,
    ) -> Result<(), Error> {
        self.send(|| upload_preview(&self.client, stream_key, thumbnail))
            .await?;
        Ok(())
    }

    pub async fn stream_preview(&self, stream_key: &str) -> Result<String, Error> {
        let version = chrono::Utc::now().timestamp_millis();
        let response = self
            .send(|| fetch_preview(&self.client, stream_key, version))
            .await?;
        let bytes = response.bytes().await.map_err(|_| Error::Decode)?;
        parse_preview(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_url_encodes_every_colon_in_the_stream_key() {
        assert_eq!(
            stream_preview_url("guild:1:2:3"),
            "https://discord.com/api/v9/streams/guild:1:2:3/preview"
        );
        assert_eq!(
            stream_preview_url("call:9:3"),
            "https://discord.com/api/v9/streams/call:9:3/preview"
        );
    }

    #[test]
    fn preview_fetch_carries_a_cache_busting_version() {
        let request = fetch_preview(&Client::new(), "guild:1:2:3", 1_700_000_000_000)
            .build()
            .unwrap();
        assert_eq!(request.method(), reqwest::Method::GET);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/streams/guild:1:2:3/preview?version=1700000000000"
        );
    }

    #[test]
    fn preview_response_returns_the_url_and_rejects_bad_json() {
        assert_eq!(
            parse_preview(br#"{"url":"https://cdn.discordapp.com/streams/guild:1:2:3/x.png"}"#)
                .unwrap(),
            "https://cdn.discordapp.com/streams/guild:1:2:3/x.png"
        );
        assert!(matches!(parse_preview(b"{}"), Err(Error::Decode)));
        assert!(matches!(parse_preview(b"not json"), Err(Error::Decode)));
    }
}
