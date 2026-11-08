use super::*;

pub(super) fn profile_query(
    kind: ProfileKind,
    guild_id: Option<u64>,
) -> Vec<(&'static str, String)> {
    let modal = kind == ProfileKind::Modal;
    let mut query = vec![
        ("type", if modal { "modal" } else { "popout" }.to_owned()),
        ("with_mutual_guilds", modal.to_string()),
        ("with_mutual_friends", modal.to_string()),
        ("with_mutual_friends_count", modal.to_string()),
    ];
    if let Some(guild_id) = guild_id {
        query.push(("guild_id", guild_id.to_string()));
    }
    query
}

impl Rest {
    pub async fn fetch_profile(
        &self,
        user_id: u64,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
        kind: ProfileKind,
    ) -> Result<model::Profile, Error> {
        let referer = channel_id.map_or_else(
            || "https://discord.com/channels/@me".to_owned(),
            |channel_id| referer(guild_id, channel_id),
        );
        let url = format!("https://discord.com/api/v9/users/{user_id}/profile");
        let query = profile_query(kind, guild_id);
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .query(&query)
                    .header(REFERER, &referer)
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn fetch_user_badges(
        &self,
        user_id: u64,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
    ) -> Result<model::UserBadges, Error> {
        let referer = channel_id.map_or_else(
            || "https://discord.com/channels/@me".to_owned(),
            |channel_id| referer(guild_id, channel_id),
        );
        let url = format!("https://discord.com/api/v9/users/{user_id}/badges");
        let response = self
            .send(|| self.client.get(&url).header(REFERER, &referer))
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn fetch_outbox(&self, user_id: u64) -> Result<model::Outbox, Error> {
        let url = format!("https://discord.com/api/v9/content-inventory/users/{user_id}/outbox");
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn fetch_content_inbox(&self) -> Result<model::ContentInbox, Error> {
        let url = "https://discord.com/api/v9/content-inventory/users/@me?for_game_profile=false&feature=inbox";
        let response = self
            .send(|| {
                self.client
                    .get(url)
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn fetch_application_identities(
        &self,
        user_id: u64,
    ) -> Result<model::ApplicationIdentities, Error> {
        let url = format!("https://discord.com/api/v9/users/{user_id}/application-identities");
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .query(&[("with_profiles", "true")])
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn fetch_guild_command_index(
        &self,
        guild_id: u64,
    ) -> Result<model::ApplicationCommandIndex, Error> {
        let url = format!("https://discord.com/api/v9/guilds/{guild_id}/application-command-index");
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn fetch_wishlist(&self, wishlist_id: u64) -> Result<model::Wishlist, Error> {
        let url = format!("https://discord.com/api/v9/wishlists/{wishlist_id}");
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .query(&[("source", "user_profile")])
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn fetch_user_affinities(&self) -> Result<Vec<model::UserAffinity>, Error> {
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/users/@me/affinities/v2/users")
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        Ok(model::parse_entries(value["user_affinities"].clone()))
    }

    pub async fn fetch_note(&self, user_id: u64) -> Result<Option<String>, Error> {
        let url = format!("https://discord.com/api/v9/users/@me/notes/{user_id}");
        let response = match self
            .send(|| {
                self.client
                    .get(&url)
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await
        {
            Ok(response) => response,
            Err(Error::Http(404)) | Err(Error::Rejected { status: 404, .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        Ok(value
            .get("note")
            .and_then(Value::as_str)
            .filter(|note| !note.is_empty())
            .map(str::to_owned))
    }

    pub async fn fetch_invite(&self, code: &str) -> Result<Option<model::Invite>, Error> {
        if code.len() < 2
            || code.len() > 32
            || !code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Ok(None);
        }
        let url = format!("https://discord.com/api/v9/invites/{code}");
        let response = match self
            .send(|| {
                self.client
                    .get(&url)
                    .query(&[("with_counts", "true"), ("with_expiration", "true")])
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await
        {
            Ok(response) => response,
            Err(Error::Http(404)) | Err(Error::Rejected { status: 404, .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        response.json().await.map_err(|_| Error::Decode).map(Some)
    }

    pub async fn fetch_games(&self, ids: &[u64]) -> Result<Vec<model::Game>, Error> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut games = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(LOOKUP_BATCH) {
            let query: Vec<_> = chunk
                .iter()
                .map(|id| ("game_ids", id.to_string()))
                .collect();
            let response = self
                .send(|| {
                    self.client
                        .get("https://discord.com/api/v9/games")
                        .query(&query)
                        .header(REFERER, "https://discord.com/channels/@me")
                })
                .await?;
            let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
            games.extend(model::parse_entries(value));
        }
        Ok(games)
    }

    pub async fn fetch_public_applications(
        &self,
        ids: &[u64],
    ) -> Result<Vec<model::PublicApplication>, Error> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut applications = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(LOOKUP_BATCH) {
            let query: Vec<_> = chunk
                .iter()
                .map(|id| ("application_ids", id.to_string()))
                .collect();
            let response = self
                .send(|| {
                    self.client
                        .get("https://discord.com/api/v9/applications/public")
                        .query(&query)
                        .header(REFERER, "https://discord.com/channels/@me")
                })
                .await?;
            let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
            applications.extend(model::parse_entries(value));
        }
        Ok(applications)
    }

    pub async fn put_widgets(
        &self,
        widgets: &[model::ProfileWidget],
    ) -> Result<Vec<model::ProfileWidget>, Error> {
        let body = serde_json::json!({
            "widgets": widgets.iter().map(model::ProfileWidget::body).collect::<Vec<_>>(),
        });
        let response = self
            .send(|| {
                self.client
                    .put("https://discord.com/api/v9/users/@me/widgets")
                    .header(REFERER, "https://discord.com/channels/@me")
                    .json(&body)
            })
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        Ok(model::parse_entries(
            value.get("widgets").cloned().unwrap_or(Value::Null),
        ))
    }

    pub async fn fetch_featured_widget_configs(
        &self,
    ) -> Result<HashMap<u64, model::WidgetConfig>, Error> {
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/widget-configs/featured")
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        Ok(model::parse_featured_widget_configs(value))
    }

    pub async fn fetch_application_widget_configs(
        &self,
        application_id: u64,
    ) -> Result<Option<model::WidgetConfig>, Error> {
        let url =
            format!("https://discord.com/api/v9/applications/{application_id}/widget-configs");
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        Ok(model::parse_application_widget_configs(
            value,
            application_id,
        ))
    }

    pub async fn fetch_collectible_product(
        &self,
        sku_id: u64,
    ) -> Result<model::CollectibleProduct, Error> {
        let url = format!("https://discord.com/api/v9/collectibles-products/{sku_id}");
        let response = self
            .send(|| {
                self.client
                    .get(&url)
                    .query(&[("locale", properties::LOCALE)])
                    .header(REFERER, "https://discord.com/channels/@me")
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }
}
