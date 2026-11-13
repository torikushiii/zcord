use super::super::*;

impl Session {
    pub async fn shop_page(&self, tab: &str) -> Result<model::ShopPage, Error> {
        self.rest(async |rest| rest.fetch_shop_page(tab).await)
            .await
    }

    pub async fn shop_products(
        &self,
        sku_ids: &[u64],
        country: Option<&str>,
    ) -> Result<Vec<model::ShopProduct>, Error> {
        self.rest(async |rest| rest.fetch_shop_products(sku_ids, country).await)
            .await
    }

    pub async fn shop_collections(
        &self,
        offset: u32,
        limit: u32,
        country: Option<&str>,
    ) -> Result<model::ShopCollections, Error> {
        self.rest(async |rest| rest.fetch_shop_collections(offset, limit, country).await)
            .await
    }

    pub async fn shop_collection(
        &self,
        collection_id: u64,
        country: Option<&str>,
    ) -> Result<Option<model::ShopCollection>, Error> {
        self.rest(async |rest| rest.fetch_shop_collection(collection_id, country).await)
            .await
    }

    pub async fn search_shop(
        &self,
        query: &model::ShopQuery,
    ) -> Result<model::ShopSearchPage, Error> {
        self.rest(async |rest| rest.search_shop(query).await).await
    }

    pub async fn orb_balance(&self) -> Result<u64, Error> {
        self.rest(async |rest| rest.fetch_orb_balance().await).await
    }

    pub async fn billing_country(&self) -> Result<Option<String>, Error> {
        self.rest(async |rest| rest.fetch_billing_country().await)
            .await
    }

    pub async fn owned_shop_skus(&self) -> Result<Vec<u64>, Error> {
        self.rest(async |rest| rest.fetch_owned_skus().await).await
    }
}
