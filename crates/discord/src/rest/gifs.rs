use super::*;
use crate::gifs;

fn media_format() -> &'static str {
    if cfg!(target_os = "linux") {
        "tinywebp"
    } else {
        "webm"
    }
}

impl Rest {
    pub async fn fetch_gif_search(&self, query: &str) -> Result<Vec<gifs::Gif>, Error> {
        let value = self
            .fetch_gif_json(
                "https://discord.com/api/v9/gifs/search",
                &[
                    ("q", query),
                    ("media_format", media_format()),
                    ("locale", properties::LOCALE),
                ],
            )
            .await?;
        Ok(gifs::klipy_gifs(&value))
    }

    pub async fn fetch_gif_trending(&self) -> Result<Vec<gifs::Gif>, Error> {
        let value = self
            .fetch_gif_json(
                "https://discord.com/api/v9/gifs/trending-gifs",
                &[
                    ("media_format", media_format()),
                    ("locale", properties::LOCALE),
                ],
            )
            .await?;
        Ok(gifs::klipy_gifs(&value))
    }

    pub async fn fetch_gif_categories(&self) -> Result<gifs::GifCategories, Error> {
        let value = self
            .fetch_gif_json(
                "https://discord.com/api/v9/gifs/trending",
                &[
                    ("locale", properties::LOCALE),
                    ("media_format", media_format()),
                ],
            )
            .await?;
        Ok(gifs::klipy_categories(&value))
    }

    pub async fn select_gif(&self, id: &str, query: &str) -> Result<(), Error> {
        let body = serde_json::json!({"id": id, "q": query});
        self.send(|| {
            self.client
                .post("https://discord.com/api/v9/gifs/select")
                .header(REFERER, "https://discord.com/channels/@me")
                .json(&body)
        })
        .await?;
        Ok(())
    }

    async fn fetch_gif_json(&self, url: &str, query: &[(&str, &str)]) -> Result<Value, Error> {
        let response = self
            .send(|| {
                self.client
                    .get(url)
                    .query(query)
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }
}
