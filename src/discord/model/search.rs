use super::{Message, parse_entries};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

#[derive(Deserialize)]
pub struct SearchPage {
    #[serde(default, rename = "total_results")]
    pub total: u64,
    #[serde(default, deserialize_with = "matching_messages")]
    pub messages: Vec<Message>,
}

fn matching_messages<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Message>, D::Error> {
    let Value::Array(hits) = Value::deserialize(deserializer)? else {
        return Ok(Vec::new());
    };
    let firsts = hits
        .into_iter()
        .filter_map(|hit| match hit {
            Value::Array(entries) => entries.into_iter().next(),
            _ => None,
        })
        .collect();
    Ok(parse_entries(Value::Array(firsts)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_page_uses_only_the_first_message_and_skips_empty_hits() {
        let page: SearchPage = serde_json::from_value(serde_json::json!({
            "total_results": 3,
            "messages": [
                [message(1), message(9)],
                [],
                [message(2)]
            ]
        }))
        .unwrap();

        assert_eq!(page.total, 3);
        assert_eq!(
            page.messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            [1, 2]
        );
    }

    #[test]
    fn search_page_defaults_absent_fields() {
        let page: SearchPage = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(page.total, 0);
        assert!(page.messages.is_empty());
    }

    #[test]
    fn search_page_skips_malformed_hits_without_promoting_context_messages() {
        let page: SearchPage = serde_json::from_value(serde_json::json!({
            "total_results": 5,
            "messages": [
                null,
                {},
                [null, message(9)],
                [{"id": "invalid"}, message(8)],
                [message(1), {"id": "invalid"}],
                [message(2)]
            ]
        }))
        .unwrap();
        assert_eq!(page.total, 5);
        assert_eq!(
            page.messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        for messages in [Value::Null, serde_json::json!({})] {
            let page: SearchPage = serde_json::from_value(serde_json::json!({
                "messages": messages
            }))
            .unwrap();
            assert!(page.messages.is_empty());
        }
    }

    fn message(id: u64) -> Value {
        serde_json::json!({
            "id": id.to_string(),
            "channel_id": "10",
            "author": {"id": "11"},
            "timestamp": "2024-01-01T00:00:00.000Z"
        })
    }
}
