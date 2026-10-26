use super::*;
use serde::{Deserialize, Deserializer, de::DeserializeOwned};
use serde_json::Value;

pub fn snowflake<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let value = Value::deserialize(d)?;
    id_value(&value).ok_or_else(|| serde::de::Error::custom("invalid snowflake"))
}

pub fn id_value(value: &Value) -> Option<u64> {
    value
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| value.as_u64())
}

pub fn optional_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    Ok(id_value(&Value::deserialize(d)?))
}

pub fn optional_nonce<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(text) => text.parse().ok(),
        Value::Number(number) => number.as_u64(),
        _ => None,
    })
}

pub fn nullable_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<u64>>, D::Error> {
    optional_id(d).map(Some)
}

pub fn nullable_string<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

pub fn parse_entries<T: DeserializeOwned>(value: Value) -> Vec<T> {
    let Value::Array(entries) = value else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter_map(|entry| match serde_json::from_value(entry) {
            Ok(entry) => Some(entry),
            Err(_) => {
                log::warn!("Skipping malformed {} entry", std::any::type_name::<T>());
                None
            }
        })
        .collect()
}

pub fn entries<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    Value::deserialize(d).map(parse_entries)
}

pub fn optional_entries<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    d: D,
) -> Result<Option<Vec<T>>, D::Error> {
    Option::<Value>::deserialize(d).map(|v| v.map(parse_entries))
}

pub fn ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(id_value)
        .collect())
}

pub fn optional_ids<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u64>>, D::Error> {
    ids(d).map(Some)
}

// `wishlist_settings` is keyed by wishlist id ("1191169174274048062"), not an array.
pub fn wishlist_ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_object()
        .into_iter()
        .flat_map(|settings| settings.keys())
        .filter_map(|id| id.parse().ok())
        .collect())
}

pub fn default_value<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    d: D,
) -> Result<T, D::Error> {
    Ok(serde_json::from_value(Value::deserialize(d)?).unwrap_or_default())
}

pub fn nullable<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(if value.is_null() {
        Some(None)
    } else {
        serde_json::from_value(value).ok().map(Some)
    })
}

pub fn lenient_message<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Message>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(serde_json::from_value(value).ok())
}

pub fn timestamp<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    chrono::DateTime::parse_from_rfc3339(&value)
        .map_err(|_| serde::de::Error::custom("invalid timestamp"))?;
    Ok(value)
}

pub fn optional_timestamp<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_str()
        .filter(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok())
        .map(str::to_owned))
}

pub fn default_true() -> bool {
    true
}

pub fn entry_groups<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    d: D,
) -> Result<Vec<Vec<T>>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .cloned()
        .map(parse_entries)
        .collect())
}
