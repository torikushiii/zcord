use super::*;

const QUEST_HOME_BANNER_DESKTOP: u32 = 3;

impl Rest {
    pub async fn fetch_quests(&self) -> Result<model::QuestList, Error> {
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/quests/@me")
                    .header(REFERER, "https://discord.com/quest-home")
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn fetch_quest_home_hero(&self) -> Result<Option<model::QuestHomeHero>, Error> {
        let response = self
            .send(|| {
                self.client
                    .get(format!(
                        "https://discord.com/api/v9/quests/get-decisions?placement={QUEST_HOME_BANNER_DESKTOP}&num_decisions_requested=1"
                    ))
                    .header(REFERER, "https://discord.com/quest-home")
            })
            .await?;
        let decisions: model::QuestDecisions = response.json().await.map_err(|_| Error::Decode)?;
        Ok(decisions
            .decisions
            .into_iter()
            .next()
            .and_then(|decision| decision.creative)
            .filter(|creative| {
                creative.creative_type.as_u64() == Some(model::QuestCreative::QUEST_HOME_HERO)
            })
            .and_then(|creative| serde_json::from_value(creative.creative_content).ok()))
    }
}
