use super::super::*;

impl Session {
    pub async fn quests(&self) -> Result<model::QuestList, Error> {
        self.rest(async |rest| rest.fetch_quests().await).await
    }

    pub async fn quest_home_hero(&self) -> Result<Option<model::QuestHomeHero>, Error> {
        self.rest(async |rest| rest.fetch_quest_home_hero().await)
            .await
    }
}
