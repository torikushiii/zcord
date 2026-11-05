use super::*;
use crate::gifs;

impl Rest {
    pub async fn fetch_gif_search(&self, query: &str) -> Result<Vec<gifs::Gif>, Error> {
        let value = self
            .fetch_gif_json(
                "https://discord.com/api/v9/gifs/search",
                &[
                    ("q", query),
                    ("media_format", "webp"),
                    ("locale", properties::LOCALE),
                    ("limit", "50"),
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
                    ("media_format", "webp"),
                    ("locale", properties::LOCALE),
                    ("limit", "50"),
                ],
            )
            .await?;
        Ok(gifs::klipy_gifs(&value))
    }

    pub async fn fetch_gif_categories(&self) -> Result<gifs::GifCategories, Error> {
        let value = self
            .fetch_gif_json(
                "https://discord.com/api/v9/gifs/trending",
                &[("media_format", "webp"), ("locale", properties::LOCALE)],
            )
            .await?;
        Ok(gifs::klipy_categories(&value))
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
