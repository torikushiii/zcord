use super::*;
use std::{sync::Mutex, time::Instant};

const QUEST_HOME_BANNER_DESKTOP: u32 = 3;
const UNKNOWN_CONNECTION_CONTEXT: &str = "eyJjb25uZWN0aW9uX3R5cGUiOiJ1bmtub3duIn0=";
const AD_SESSION_IDLE: Duration = Duration::from_secs(30 * 60);
const AD_SESSION_LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);

static AD_SESSION: Mutex<Option<AdSession>> = Mutex::new(None);

struct AdSession {
    id: String,
    created: Instant,
    used: Instant,
}

impl AdSession {
    fn new(now: Instant) -> Self {
        Self {
            id: properties::uuid_v4(),
            created: now,
            used: now,
        }
    }

    fn expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.used) > AD_SESSION_IDLE
            || now.saturating_duration_since(self.created) > AD_SESSION_LIFETIME
    }
}

fn ad_session_id(now: Instant) -> String {
    let mut session = AD_SESSION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let session = match session.as_mut() {
        Some(current) if !current.expired(now) => current,
        _ => session.insert(AdSession::new(now)),
    };
    session.used = now;
    session.id.clone()
}

fn decisions_query(heartbeat: Option<&str>, ad_session: &str) -> Vec<(&'static str, String)> {
    let mut query = vec![
        ("placement", QUEST_HOME_BANNER_DESKTOP.to_string()),
        ("num_decisions_requested", "1".to_owned()),
    ];
    query.extend(heartbeat.map(|id| ("client_heartbeat_session_id", id.to_owned())));
    query.push(("client_ad_session_id", ad_session.to_owned()));
    query
}

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
        let properties = properties::client_properties();
        let query = decisions_query(
            properties["client_heartbeat_session_id"].as_str(),
            &ad_session_id(Instant::now()),
        );
        let response = self
            .send(|| {
                self.client
                    .get("https://discord.com/api/v9/quests/get-decisions")
                    .query(&query)
                    .header(REFERER, "https://discord.com/quest-home")
                    .header("x-context-properties", UNKNOWN_CONNECTION_CONTEXT)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_query_carries_both_session_ids() {
        let query = decisions_query(Some("h"), "a")
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("&");
        assert_eq!(
            query,
            "placement=3&num_decisions_requested=1&client_heartbeat_session_id=h&client_ad_session_id=a"
        );
    }

    #[test]
    fn ad_session_rotates_after_idle_or_lifetime() {
        let start = Instant::now();
        let mut session = AdSession::new(start);
        assert!(!session.expired(start + AD_SESSION_IDLE));
        assert!(session.expired(start + AD_SESSION_IDLE + Duration::from_secs(1)));
        session.used = start + AD_SESSION_LIFETIME;
        assert!(session.expired(start + AD_SESSION_LIFETIME + Duration::from_secs(1)));
    }
}
