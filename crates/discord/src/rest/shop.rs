use super::*;
use serde::Deserialize;

const REFERER_SHOP: &str = "https://discord.com/shop";
const COLLECTIBLES_APPLICATION: &str = "1096190356233670716";
const LOCALE: &str = "en-US";

fn owned_skus(purchases: Vec<Value>) -> Vec<u64> {
    #[derive(Deserialize)]
    struct Item {
        #[serde(default, deserialize_with = "model::optional_id")]
        sku_id: Option<u64>,
    }
    #[derive(Deserialize)]
    struct Purchase {
        #[serde(default, deserialize_with = "model::optional_id")]
        sku_id: Option<u64>,
        #[serde(default, deserialize_with = "model::entries")]
        items: Vec<Item>,
    }
    let mut skus = Vec::new();
    for purchase in purchases {
        let Ok(purchase) = serde_json::from_value::<Purchase>(purchase) else {
            continue;
        };
        skus.extend(purchase.sku_id);
        skus.extend(purchase.items.into_iter().filter_map(|item| item.sku_id));
    }
    skus.sort_unstable();
    skus.dedup();
    skus
}

fn with_country(country: Option<&str>) -> Vec<(&'static str, String)> {
    country
        .map(|country| ("country_code", country.to_owned()))
        .into_iter()
        .collect()
}

fn pairs(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    pairs
        .iter()
        .map(|(key, value)| (*key, (*value).to_owned()))
        .collect()
}

async fn decode<T: Send + 'static>(
    response: Response,
    parse: impl FnOnce(Value) -> T + Send + 'static,
) -> Result<T, Error> {
    let bytes = response.bytes().await.map_err(|_| Error::Network)?;
    tokio::task::spawn_blocking(move || {
        serde_json::from_slice::<Value>(&bytes)
            .map(parse)
            .map_err(|_| Error::Decode)
    })
    .await
    .map_err(|_| Error::Decode)?
}

impl Rest {
    pub async fn fetch_shop_page(&self, tab: &str) -> Result<model::ShopPage, Error> {
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/collectibles-shop")
                    .query(&[
                        ("tab", tab),
                        ("include_bundles", "true"),
                        ("include_dynamic_blocks", "true"),
                        ("variants_return_style", "2"),
                        ("skip_num_categories", "0"),
                    ])
                    .header(REFERER, REFERER_SHOP)
            })
            .await?;
        decode(response, serde_json::from_value::<model::ShopPage>)
            .await?
            .map_err(|_| Error::Decode)
    }

    pub async fn fetch_shop_products(
        &self,
        sku_ids: &[u64],
        country: Option<&str>,
    ) -> Result<Vec<model::ShopProduct>, Error> {
        let mut query = with_country(country);
        query.extend(sku_ids.iter().map(|id| ("sku_ids", id.to_string())));
        query.extend(pairs(&[
            ("locale", LOCALE),
            ("with_bundled_skus", "true"),
            ("include_google_sku_ids", "false"),
            ("ignore_cache", "false"),
        ]));
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/storefront/products/skus")
                    .query(&query)
                    .header(REFERER, REFERER_SHOP)
            })
            .await?;
        decode(response, model::storefront_products).await
    }

    pub async fn fetch_shop_collections(
        &self,
        offset: u32,
        limit: u32,
        country: Option<&str>,
    ) -> Result<model::ShopCollections, Error> {
        let mut query = with_country(country);
        query.extend(pairs(&[
            ("application_id", COLLECTIBLES_APPLICATION),
            ("use_shop_ordering", "true"),
        ]));
        query.push(("offset", offset.to_string()));
        query.push(("limit", limit.to_string()));
        query.extend(pairs(&[
            ("include_pricing", "true"),
            ("locale", LOCALE),
            ("with_bundled_skus", "true"),
            ("include_google_sku_ids", "false"),
            ("include_unpublished_products", "false"),
            ("include_unpublished_collections", "false"),
            ("ignore_cache", "false"),
        ]));
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/storefront/collections/applications")
                    .query(&query)
                    .header(REFERER, REFERER_SHOP)
            })
            .await?;
        decode(response, model::storefront_collections).await
    }

    pub async fn fetch_shop_collection(
        &self,
        collection_id: u64,
        country: Option<&str>,
    ) -> Result<Option<model::ShopCollection>, Error> {
        let mut query = with_country(country);
        query.push(("collection_ids", collection_id.to_string()));
        query.extend(pairs(&[
            ("locale", LOCALE),
            ("with_bundled_skus", "true"),
            ("include_pricing", "true"),
            ("include_google_sku_ids", "false"),
            ("include_unpublished_products", "false"),
            ("include_unpublished_collections", "false"),
        ]));
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/storefront/collections")
                    .query(&query)
                    .header(REFERER, REFERER_SHOP)
            })
            .await?;
        let collections = decode(response, model::storefront_collections).await?;
        Ok(collections
            .collections
            .into_iter()
            .find(|collection| collection.id == collection_id))
    }

    pub async fn search_shop(
        &self,
        query: &model::ShopQuery,
    ) -> Result<model::ShopSearchPage, Error> {
        let params = query.params();
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/shop/search")
                    .query(&params)
                    .header(REFERER, REFERER_SHOP)
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn fetch_orb_balance(&self) -> Result<u64, Error> {
        #[derive(Deserialize)]
        struct Balance {
            #[serde(default)]
            balance: u64,
        }
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/users/@me/virtual-currency/balance")
                    .header(REFERER, REFERER_SHOP)
            })
            .await?;
        let balance: Balance = response.json().await.map_err(|_| Error::Decode)?;
        Ok(balance.balance)
    }

    pub async fn fetch_billing_country(&self) -> Result<Option<String>, Error> {
        #[derive(Deserialize)]
        struct Country {
            #[serde(default)]
            country_code: Option<String>,
        }
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/users/@me/billing/country-code")
                    .header(REFERER, REFERER_SHOP)
            })
            .await?;
        let country: Country = response.json().await.map_err(|_| Error::Decode)?;
        Ok(country
            .country_code
            .filter(|code| code.len() == 2 && code.bytes().all(|b| b.is_ascii_alphabetic())))
    }

    pub async fn fetch_owned_skus(&self) -> Result<Vec<u64>, Error> {
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/users/@me/collectibles-purchases")
                    .query(&[("variants_return_style", "2")])
                    .header(REFERER, REFERER_SHOP)
            })
            .await?;
        let purchases: Vec<Value> = response.json().await.map_err(|_| Error::Decode)?;
        Ok(owned_skus(purchases))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn owned_skus_collect_products_and_their_items() {
        let purchases = vec![
            json!({"sku_id": "30", "items": [{"sku_id": "31"}, {"sku_id": "32"}]}),
            json!({"sku_id": "10", "items": [{"sku_id": "10"}]}),
            json!({"sku_id": "badge"}),
            json!("garbage"),
        ];
        assert_eq!(owned_skus(purchases), [10, 30, 31, 32]);
    }
}
