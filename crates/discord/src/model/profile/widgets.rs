use super::*;

#[derive(Clone, Debug)]
pub struct ProfileWidget {
    pub id: u64,
    pub data: WidgetData,
    /// The widget's `data` as Discord sent it. Saving replaces the whole list, so widgets this
    /// client can't fully model are sent back exactly as they came.
    pub raw: Value,
}

impl<'de> Deserialize<'de> for ProfileWidget {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Id {
            #[serde(deserialize_with = "snowflake")]
            id: u64,
        }
        let value = Value::deserialize(d)?;
        let Id { id } = Id::deserialize(&value).map_err(serde::de::Error::custom)?;
        let raw = value.get("data").cloned().unwrap_or(Value::Null);
        // A widget whose data no longer parses must still survive a save, so it falls back to
        // `Unknown` rather than being dropped from the list.
        let data = WidgetData::deserialize(&raw).unwrap_or(WidgetData::Unknown);
        Ok(Self { id, data, raw })
    }
}

impl ProfileWidget {
    /// The `{id, data}` entry Discord's `PUT /users/@me/widgets` takes for this widget. Only an
    /// edited game list is rewritten, and each game keeps the fields Discord sent for it.
    pub fn body(&self) -> Value {
        let mut data = self.raw.clone();
        let original = WidgetData::deserialize(&self.raw).ok();
        if let Some(games) = self.data.games()
            && original.as_ref().and_then(WidgetData::games) != Some(games)
            && let Some(object) = data.as_object_mut()
        {
            let sent = object
                .get("games")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let games: Vec<Value> = games
                .iter()
                .map(|game| {
                    let mut entry = sent
                        .iter()
                        .find(|entry| match entry.get("game_id") {
                            Some(Value::String(id)) => id.parse() == Ok(game.game_id),
                            Some(id) => id.as_u64() == Some(game.game_id),
                            None => false,
                        })
                        .cloned()
                        .filter(Value::is_object)
                        .unwrap_or_else(|| serde_json::json!({}));
                    entry["game_id"] = game.game_id.to_string().into();
                    entry["comment"] = serde_json::json!(game.comment);
                    entry["tags"] = serde_json::json!(game.tags);
                    entry
                })
                .collect();
            object.insert("games".into(), Value::Array(games));
        }
        serde_json::json!({ "id": self.id.to_string(), "data": data })
    }
}

impl WidgetData {
    pub fn games(&self) -> Option<&Vec<WidgetGame>> {
        match self {
            Self::FavoriteGames { games }
            | Self::CurrentGames { games }
            | Self::PlayedGames { games }
            | Self::WantToPlayGames { games } => Some(games),
            _ => None,
        }
    }

    pub fn games_mut(&mut self) -> Option<&mut Vec<WidgetGame>> {
        match self {
            Self::FavoriteGames { games }
            | Self::CurrentGames { games }
            | Self::PlayedGames { games }
            | Self::WantToPlayGames { games } => Some(games),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WidgetData {
    FavoriteGames {
        #[serde(default, deserialize_with = "default_value")]
        games: Vec<WidgetGame>,
    },
    CurrentGames {
        #[serde(default, deserialize_with = "default_value")]
        games: Vec<WidgetGame>,
    },
    /// The profile Board labels this list "Games I Like".
    PlayedGames {
        #[serde(default, deserialize_with = "default_value")]
        games: Vec<WidgetGame>,
    },
    WantToPlayGames {
        #[serde(default, deserialize_with = "default_value")]
        games: Vec<WidgetGame>,
    },
    Application {
        #[serde(deserialize_with = "snowflake")]
        application_id: u64,
    },
    Personal {
        #[serde(default, deserialize_with = "empty_string_none")]
        header: Option<String>,
        #[serde(default, deserialize_with = "entries")]
        sections: Vec<PersonalSection>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PersonalSection {
    Cover {
        #[serde(default, deserialize_with = "empty_string_none")]
        title: Option<String>,
        #[serde(default, deserialize_with = "empty_string_none")]
        subtitle: Option<String>,
        #[serde(default, deserialize_with = "default_value")]
        image: Option<PersonalImage>,
    },
    Fields {
        #[serde(default, deserialize_with = "personal_fields")]
        fields: Vec<PersonalField>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PersonalField {
    #[serde(default, deserialize_with = "empty_string_none")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "empty_string_none")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub image: Option<PersonalImage>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PersonalImage {
    #[serde(deserialize_with = "snowflake")]
    pub file_id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub width: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub height: Option<u32>,
    #[serde(default)]
    pub is_animated: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct WidgetGame {
    #[serde(deserialize_with = "snowflake")]
    pub game_id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub comment: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub tags: Vec<String>,
}

pub(super) fn steam_app_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .find(|sku| sku.get("distributor").and_then(Value::as_str) == Some("steam"))
        .and_then(|sku| sku.get("id"))
        .and_then(|id| match id {
            Value::String(id) => Some(id.clone()),
            Value::Number(id) => Some(id.to_string()),
            _ => None,
        }))
}

fn empty_string_none<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned))
}

fn personal_fields<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<PersonalField>, D::Error> {
    let value = Value::deserialize(d)?;
    let Value::Array(entries) = value else {
        return Err(serde::de::Error::custom("personal fields must be an array"));
    };
    Ok(parse_entries(Value::Array(entries)))
}
