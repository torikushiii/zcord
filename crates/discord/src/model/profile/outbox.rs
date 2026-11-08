use super::*;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Outbox {
    #[serde(default, deserialize_with = "entries")]
    pub entries: Vec<OutboxEntry>,
}

#[derive(Clone, Debug)]
pub struct OutboxEntry {
    pub id: u64,
    pub author_id: Option<u64>,
    pub content_type: u8,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub expires_at: Option<String>,
    pub kind: OutboxKind,
    pub traits: Vec<OutboxTrait>,
    pub participants: Vec<u64>,
    pub raw: std::sync::Arc<Value>,
}

impl<'de> Deserialize<'de> for OutboxEntry {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(deserialize_with = "snowflake")]
            id: u64,
            #[serde(default, deserialize_with = "optional_id")]
            author_id: Option<u64>,
            #[serde(default, deserialize_with = "default_value")]
            content_type: u8,
            #[serde(default, deserialize_with = "default_value")]
            started_at: Option<String>,
            #[serde(default, deserialize_with = "default_value")]
            ended_at: Option<String>,
            #[serde(default, deserialize_with = "default_value")]
            expires_at: Option<String>,
            #[serde(default)]
            extra: Value,
            #[serde(default)]
            traits: Value,
        }
        let value = Value::deserialize(d)?;
        let raw = Raw::deserialize(&value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            id: raw.id,
            author_id: raw.author_id,
            content_type: raw.content_type,
            started_at: raw.started_at,
            ended_at: raw.ended_at,
            expires_at: raw.expires_at,
            kind: outbox_kind(&raw.extra),
            traits: outbox_traits(&raw.traits),
            participants: value
                .get("participants")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(id_value)
                .collect(),
            raw: std::sync::Arc::new(value),
        })
    }
}

#[derive(Clone, Debug)]
pub enum OutboxKind {
    PlayedGame {
        name: String,
        application_id: Option<u64>,
    },
    Listened {
        tracks: Vec<ListenedTrack>,
        last_update: Option<String>,
    },
    LaunchedActivity {
        name: String,
        application_id: Option<u64>,
        last_update: Option<String>,
    },
    Unknown,
}

#[derive(Clone, Debug)]
pub struct ListenedTrack {
    pub title: String,
    pub artists: Vec<String>,
    pub image_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OutboxTrait {
    Live,
    Duration { seconds: u64 },
    FirstTime,
    AggregateRange { range: u8 },
    Resurrected { last_played: String },
    Marathon,
    NewRelease,
    Streak { days: u32 },
    Trending,
    TopItemCount { count: u32 },
    TopParentCount { count: u32 },
    AggregateCount { count: u32 },
    Unknown,
}

fn outbox_kind(extra: &Value) -> OutboxKind {
    match extra.get("type").and_then(Value::as_str) {
        Some("played_game_extra") => OutboxKind::PlayedGame {
            name: extra
                .get("game_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            application_id: extra.get("application_id").and_then(id_value),
        },
        Some("launched_activity_extra") => OutboxKind::LaunchedActivity {
            name: extra
                .get("activity_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            application_id: extra.get("application_id").and_then(id_value),
            last_update: extra
                .get("last_update")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        Some("listened_session_extra") => OutboxKind::Listened {
            last_update: extra
                .get("last_update")
                .and_then(Value::as_str)
                .map(str::to_owned),
            tracks: extra
                .get("entries")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(listened_track)
                .collect(),
        },
        _ => OutboxKind::Unknown,
    }
}

fn listened_track(entry: &Value) -> Option<ListenedTrack> {
    let media = entry.get("media")?;
    Some(ListenedTrack {
        title: media.get("title").and_then(Value::as_str)?.to_owned(),
        artists: media
            .get("artists")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|artist| {
                artist
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect(),
        image_url: media
            .get("image_url")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn outbox_traits(traits: &Value) -> Vec<OutboxTrait> {
    traits
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|trait_value| {
            let kind = trait_value.get("type")?.as_u64()?;
            Some(match kind {
                1 if trait_value.get("first_time").and_then(Value::as_bool) == Some(true) => {
                    OutboxTrait::FirstTime
                }
                2 => trait_value
                    .get("duration_seconds")
                    .and_then(Value::as_u64)
                    .map(|seconds| OutboxTrait::Duration { seconds })
                    .unwrap_or(OutboxTrait::Unknown),
                3 if trait_value.get("is_live").and_then(Value::as_bool) == Some(true) => {
                    OutboxTrait::Live
                }
                4 => trait_value
                    .get("range")
                    .and_then(Value::as_u64)
                    .and_then(|range| u8::try_from(range).ok())
                    .map(|range| OutboxTrait::AggregateRange { range })
                    .unwrap_or(OutboxTrait::Unknown),
                5 => trait_value
                    .get("resurrected_last_played")
                    .and_then(Value::as_str)
                    .map(|last_played| OutboxTrait::Resurrected {
                        last_played: last_played.to_owned(),
                    })
                    .unwrap_or(OutboxTrait::Unknown),
                6 if trait_value.get("marathon").and_then(Value::as_bool) == Some(true) => {
                    OutboxTrait::Marathon
                }
                7 => OutboxTrait::NewRelease,
                8 => trait_value
                    .get("streak_count_days")
                    .and_then(Value::as_u64)
                    .and_then(|days| u32::try_from(days).ok())
                    .map(|days| OutboxTrait::Streak { days })
                    .unwrap_or(OutboxTrait::Unknown),
                9 => OutboxTrait::Trending,
                10 => trait_value
                    .get("count")
                    .and_then(Value::as_u64)
                    .and_then(|count| u32::try_from(count).ok())
                    .map(|count| OutboxTrait::TopItemCount { count })
                    .unwrap_or(OutboxTrait::Unknown),
                11 => trait_value
                    .get("count")
                    .and_then(Value::as_u64)
                    .and_then(|count| u32::try_from(count).ok())
                    .map(|count| OutboxTrait::TopParentCount { count })
                    .unwrap_or(OutboxTrait::Unknown),
                12 => trait_value
                    .get("count")
                    .and_then(Value::as_u64)
                    .and_then(|count| u32::try_from(count).ok())
                    .map(|count| OutboxTrait::AggregateCount { count })
                    .unwrap_or(OutboxTrait::Unknown),
                _ => OutboxTrait::Unknown,
            })
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct ContentInbox {
    pub entries: Vec<OutboxEntry>,
    pub wait_ms_until_next_fetch: Option<u64>,
}

impl<'de> Deserialize<'de> for ContentInbox {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Ranked {
            rank: i64,
            content: OutboxEntry,
        }
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default, deserialize_with = "entries")]
            entries: Vec<Ranked>,
            #[serde(default, deserialize_with = "default_value")]
            wait_ms_until_next_fetch: Option<u64>,
        }
        let raw = Raw::deserialize(d)?;
        let mut entries = raw.entries;
        entries.sort_by_key(|entry| entry.rank);
        Ok(Self {
            entries: entries.into_iter().map(|entry| entry.content).collect(),
            wait_ms_until_next_fetch: raw.wait_ms_until_next_fetch,
        })
    }
}
