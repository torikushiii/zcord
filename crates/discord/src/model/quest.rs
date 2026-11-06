use std::collections::HashMap;

use serde::Deserialize;

use super::*;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct QuestList {
    #[serde(default, deserialize_with = "entries")]
    pub quests: Vec<Quest>,
    #[serde(default, deserialize_with = "excluded_ids")]
    pub excluded_quests: Vec<u64>,
    #[serde(default)]
    pub quest_enrollment_blocked_until: Option<String>,
}

fn excluded_ids<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
    #[derive(Deserialize)]
    struct Excluded {
        #[serde(deserialize_with = "snowflake")]
        id: u64,
    }
    entries::<D, Excluded>(d).map(|entries| entries.into_iter().map(|entry| entry.id).collect())
}

#[derive(Clone, Debug, Deserialize)]
pub struct Quest {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    pub config: QuestConfig,
    #[serde(default, deserialize_with = "lenient")]
    pub user_status: Option<QuestUserStatus>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct QuestConfig {
    pub starts_at: String,
    pub expires_at: String,
    #[serde(default, deserialize_with = "lenient")]
    pub application: Option<QuestApplication>,
    #[serde(default, deserialize_with = "default_value")]
    pub assets: QuestAssets,
    #[serde(default, deserialize_with = "default_value")]
    pub colors: QuestColors,
    #[serde(default, deserialize_with = "default_value")]
    pub messages: QuestMessages,
    #[serde(default, deserialize_with = "default_value")]
    pub task_config_v2: QuestTasks,
    #[serde(default, deserialize_with = "default_value")]
    pub rewards_config: QuestRewards,
    #[serde(default, deserialize_with = "lenient")]
    pub cta_config: Option<QuestCta>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct QuestApplication {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub name: String,
    #[serde(default)]
    pub link: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct QuestAssets {
    #[serde(default, deserialize_with = "quest_asset")]
    pub hero: Option<String>,
    #[serde(default, deserialize_with = "quest_asset")]
    pub hero_video: Option<String>,
    #[serde(default, deserialize_with = "quest_asset")]
    pub quest_bar_hero: Option<String>,
    #[serde(default, deserialize_with = "quest_asset")]
    pub game_tile: Option<String>,
    #[serde(default, deserialize_with = "quest_asset")]
    pub game_tile_dark: Option<String>,
    #[serde(default, deserialize_with = "quest_asset")]
    pub logotype: Option<String>,
    #[serde(default, deserialize_with = "quest_asset")]
    pub logotype_dark: Option<String>,
}

fn lenient<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(d)?).ok())
}

fn lenient_map<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(
    d: D,
) -> Result<HashMap<String, T>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key.clone(), serde_json::from_value(value.clone()).ok()?)))
        .collect())
}

fn quest_asset<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.filter(|path| !path.is_empty() && path != "PLACEHOLDER"))
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct QuestColors {
    #[serde(default)]
    pub primary: Option<String>,
    #[serde(default)]
    pub secondary: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct QuestMessages {
    #[serde(default, deserialize_with = "default_value")]
    pub quest_name: String,
    #[serde(default, deserialize_with = "default_value")]
    pub game_title: String,
    #[serde(default, deserialize_with = "default_value")]
    pub game_publisher: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct QuestTasks {
    #[serde(default, deserialize_with = "lenient_map")]
    pub tasks: HashMap<String, QuestTask>,
    #[serde(default)]
    pub join_operator: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct QuestTask {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, deserialize_with = "default_value")]
    pub target: u64,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct QuestRewards {
    #[serde(default, deserialize_with = "entries")]
    pub rewards: Vec<QuestReward>,
    #[serde(default)]
    pub rewards_expire_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct QuestReward {
    #[serde(rename = "type")]
    pub kind: u32,
    #[serde(default, deserialize_with = "quest_asset")]
    pub asset: Option<String>,
    #[serde(default, deserialize_with = "quest_asset")]
    pub asset_video: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub messages: QuestRewardMessages,
    #[serde(default)]
    pub orb_quantity: Option<u64>,
}

impl QuestReward {
    pub const REDEEM_CODE: u32 = 1;
    pub const COLLECTIBLE: u32 = 3;
    pub const ORBS: u32 = 4;
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct QuestRewardMessages {
    #[serde(default, deserialize_with = "default_value")]
    pub name: String,
    #[serde(default)]
    pub name_with_article: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct QuestCta {
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub button_label: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct QuestUserStatus {
    #[serde(default)]
    pub enrolled_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub claimed_at: Option<String>,
    #[serde(default, deserialize_with = "lenient_map")]
    pub progress: HashMap<String, QuestProgress>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct QuestProgress {
    #[serde(default, deserialize_with = "default_value")]
    pub value: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct QuestDecisions {
    #[serde(default, deserialize_with = "entries")]
    pub decisions: Vec<QuestDecision>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct QuestDecision {
    #[serde(default, deserialize_with = "lenient")]
    pub creative: Option<QuestCreative>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct QuestCreative {
    #[serde(default)]
    pub creative_type: serde_json::Value,
    #[serde(default)]
    pub creative_content: serde_json::Value,
}

impl QuestCreative {
    pub const QUEST_HOME_HERO: u64 = 2;
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct QuestHomeHero {
    #[serde(default, deserialize_with = "default_value")]
    pub label_title: String,
    #[serde(default, deserialize_with = "default_value")]
    pub label_subtitle: String,
    #[serde(default)]
    pub hero_image: Option<String>,
    #[serde(default)]
    pub hero_video: Option<String>,
    #[serde(default)]
    pub sponsor_image: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub cta: Option<QuestHeroCta>,
    #[serde(default, deserialize_with = "lenient_ids")]
    pub quest_ids: Vec<u64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct QuestHeroCta {
    #[serde(default, deserialize_with = "default_value")]
    pub url: String,
    #[serde(default, deserialize_with = "default_value")]
    pub button_label: String,
}

fn lenient_ids<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
    let values = Option::<Vec<serde_json::Value>>::deserialize(d)?.unwrap_or_default();
    Ok(values
        .iter()
        .filter_map(|value| match value {
            serde_json::Value::String(id) => id.parse().ok(),
            serde_json::Value::Number(id) => id.as_u64(),
            _ => None,
        })
        .collect())
}
