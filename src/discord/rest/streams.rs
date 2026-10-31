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

impl Rest {
    pub async fn stream_preview(&self, stream_key: &str) -> Result<String, Error> {
        let url = stream_preview_url(stream_key);
        let response = self.send(|| self.client.get(&url)).await?;
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
