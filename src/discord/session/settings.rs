use super::super::*;

impl Session {
    pub async fn gif_search(&self, provider: GifProvider, query: &str) -> Result<Vec<Gif>, Error> {
        match provider {
            GifProvider::Klipy => {
                self.rest(async |rest| rest.fetch_gif_search(query).await)
                    .await
            }
            GifProvider::Tenor => gifs::tenor_search(self.gif_client()?, query).await,
            GifProvider::Giphy => gifs::giphy_search(self.gif_client()?, query).await,
        }
    }

    pub async fn gif_trending(&self, provider: GifProvider) -> Result<Vec<Gif>, Error> {
        match provider {
            GifProvider::Klipy => {
                self.rest(async |rest| rest.fetch_gif_trending().await)
                    .await
            }
            GifProvider::Tenor => gifs::tenor_trending(self.gif_client()?).await,
            GifProvider::Giphy => gifs::giphy_trending(self.gif_client()?).await,
        }
    }

    pub async fn gif_categories(&self, provider: GifProvider) -> Result<GifCategories, Error> {
        match provider {
            GifProvider::Klipy => {
                self.rest(async |rest| rest.fetch_gif_categories().await)
                    .await
            }
            GifProvider::Tenor => gifs::tenor_categories_request(self.gif_client()?).await,
            GifProvider::Giphy => gifs::giphy_categories_request(self.gif_client()?).await,
        }
    }

    pub async fn frecency_settings(&self) -> Result<Vec<u8>, Error> {
        self.rest(async |rest| rest.fetch_frecency_settings().await)
            .await
    }

    pub async fn update_frecency_settings(&self, proto: &[u8]) -> Result<Vec<u8>, Error> {
        self.rest(async |rest| rest.update_frecency_settings(proto).await)
            .await
    }
}
