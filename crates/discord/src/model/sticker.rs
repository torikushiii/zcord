use super::serde_helpers::*;
use serde::Deserialize;

/// Full sticker metadata, separate from the minimal message sticker item.
#[derive(Clone, Deserialize)]
pub struct StickerDetails {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Discord's comma-separated autocomplete tags.
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub format_type: u8,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default, deserialize_with = "optional_id")]
    pub pack_id: Option<u64>,
    /// Discord sticker type: 1 = standard, 2 = guild, 0 = unknown.
    #[serde(rename = "type", default, deserialize_with = "default_value")]
    pub kind: u8,
    /// `None` means availability was not reported.
    #[serde(default)]
    pub available: Option<bool>,
}

/// External guild metadata, kept separate from the joined-guild READY model.
#[derive(Clone, Deserialize)]
pub struct StickerGuild {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub features: Option<Vec<String>>,
    #[serde(default, deserialize_with = "default_value")]
    pub approximate_presence_count: Option<u32>,
    /// Discovery responses may include at most 30 stickers, not the full catalog.
    #[serde(default, deserialize_with = "optional_entries")]
    pub stickers: Option<Vec<StickerDetails>>,
    /// Reported total, when present; do not infer it from the embedded catalog.
    #[serde(default, deserialize_with = "default_value")]
    pub sticker_count: Option<u32>,
}

impl StickerGuild {
    /// Missing features remain unknown rather than implying an invite-only guild.
    pub fn discoverable(&self) -> Option<bool> {
        self.features
            .as_ref()
            .map(|features| features.iter().any(|feature| feature == "DISCOVERABLE"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sticker_details_decode_string_snowflakes_and_type() {
        let sticker: StickerDetails = serde_json::from_value(json!({
            "id": "9007199254740993",
            "name": "wave",
            "description": "A greeting",
            "tags": "hello,wave",
            "format_type": 4,
            "guild_id": "200",
            "pack_id": "9007199254740995",
            "type": 2,
            "available": false
        }))
        .unwrap();
        assert_eq!(sticker.id, 9007199254740993);
        assert_eq!(sticker.name, "wave");
        assert_eq!(sticker.description.as_deref(), Some("A greeting"));
        assert_eq!(sticker.tags.as_deref(), Some("hello,wave"));
        assert_eq!(sticker.format_type, 4);
        assert_eq!(sticker.guild_id, Some(200));
        assert_eq!(sticker.pack_id, Some(9007199254740995));
        assert_eq!(sticker.kind, 2);
        assert_eq!(sticker.available, Some(false));
    }

    #[test]
    fn sticker_details_default_missing_and_null_metadata() {
        for payload in [
            json!({"id": "10"}),
            json!({
                "id": 10,
                "name": null,
                "description": null,
                "tags": null,
                "format_type": null,
                "guild_id": null,
                "pack_id": null,
                "type": null,
                "available": null
            }),
        ] {
            let sticker: StickerDetails = serde_json::from_value(payload).unwrap();
            assert_eq!(sticker.id, 10);
            assert!(sticker.name.is_empty());
            assert!(sticker.description.is_none());
            assert!(sticker.tags.is_none());
            assert_eq!(sticker.format_type, 0);
            assert!(sticker.guild_id.is_none());
            assert!(sticker.pack_id.is_none());
            assert_eq!(sticker.kind, 0);
            assert!(sticker.available.is_none());
        }
    }

    #[test]
    fn sticker_details_decode_standard_and_numeric_ids() {
        let sticker: StickerDetails = serde_json::from_value(json!({
            "id": 10, "pack_id": 20, "type": 1, "format_type": 3
        }))
        .unwrap();
        assert_eq!(sticker.kind, 1);
        assert_eq!(sticker.pack_id, Some(20));
        assert!(sticker.guild_id.is_none());
        let sticker: StickerDetails = serde_json::from_value(json!({
            "id": 10, "guild_id": 200, "pack_id": "bad", "available": true
        }))
        .unwrap();
        assert_eq!(sticker.guild_id, Some(200));
        assert!(sticker.pack_id.is_none());
        assert_eq!(sticker.available, Some(true));
    }

    #[test]
    fn sticker_details_require_a_valid_identity() {
        for payload in [json!({}), json!({"id": null}), json!({"id": "bad"})] {
            assert!(serde_json::from_value::<StickerDetails>(payload).is_err());
        }
    }

    #[test]
    fn sticker_guild_decodes_flat_discovery_metadata_and_partial_catalog() {
        // Synthetic payload following the public discoverable-guild schema.
        let guild: StickerGuild = serde_json::from_value(json!({
            "id": "9007199254740993",
            "name": "External guild",
            "icon": "a_icon_hash",
            "features": ["COMMUNITY", "DISCOVERABLE"],
            "approximate_member_count": 150000,
            "approximate_presence_count": 21125,
            "auto_removed": false,
            "sticker_count": 45,
            "stickers": [
                {"id": "9007199254740995", "name": "wave", "format_type": 2,
                 "type": 2, "guild_id": "9007199254740993", "description": null},
                {"id": "9007199254740997", "name": "dance", "format_type": 4},
                {"id": "bad"}, {"name": "missing-id"}, null
            ]
        }))
        .unwrap();
        assert_eq!(guild.id, 9007199254740993);
        assert_eq!(guild.name, "External guild");
        assert_eq!(guild.icon.as_deref(), Some("a_icon_hash"));
        assert_eq!(guild.discoverable(), Some(true));
        assert_eq!(guild.approximate_presence_count, Some(21125));
        assert_eq!(guild.sticker_count, Some(45));
        let stickers = guild.stickers.as_ref().unwrap();
        assert_eq!(stickers.len(), 2);
        assert_eq!(stickers[0].id, 9007199254740995);
        assert_eq!(stickers[0].name, "wave");
        assert_eq!(stickers[0].format_type, 2);
        assert_eq!(stickers[0].guild_id, Some(guild.id));
        assert_eq!(stickers[1].id, 9007199254740997);
        assert_eq!(stickers[1].format_type, 4);
    }

    #[test]
    fn sticker_guild_preserves_unknown_optional_metadata() {
        for payload in [
            json!({"id": "200"}),
            json!({
                "id": 200, "name": null, "icon": null, "features": null,
                "approximate_presence_count": null, "stickers": null, "sticker_count": null
            }),
        ] {
            let guild: StickerGuild = serde_json::from_value(payload).unwrap();
            assert_eq!(guild.id, 200);
            assert!(guild.name.is_empty());
            assert!(guild.icon.is_none());
            assert!(guild.features.is_none());
            assert_eq!(guild.discoverable(), None);
            assert!(guild.approximate_presence_count.is_none());
            assert!(guild.stickers.is_none());
            assert!(guild.sticker_count.is_none());
        }
    }

    #[test]
    fn sticker_guild_decodes_preview_shape_and_known_empty_catalog() {
        let guild: StickerGuild = serde_json::from_value(json!({
            "id": 200, "name": "Preview guild", "icon": null,
            "splash": null, "discovery_splash": null, "description": null,
            "features": [], "emojis": [], "approximate_member_count": 10,
            "approximate_presence_count": 0, "stickers": []
        }))
        .unwrap();
        assert_eq!(guild.id, 200);
        assert_eq!(guild.discoverable(), Some(false));
        assert_eq!(guild.approximate_presence_count, Some(0));
        assert!(guild.stickers.as_ref().unwrap().is_empty());
        assert!(guild.sticker_count.is_none());
        let guild: StickerGuild = serde_json::from_value(json!({
            "id": "200", "features": ["COMMUNITY"], "sticker_count": 0
        }))
        .unwrap();
        assert_eq!(guild.discoverable(), Some(false));
        assert_eq!(guild.sticker_count, Some(0));
        assert!(guild.stickers.is_none());
    }

    #[test]
    fn sticker_guild_counts_are_optional_checked_integers() {
        for (value, expected) in [
            (json!(0), Some(0)),
            (json!(u32::MAX), Some(u32::MAX)),
            (json!(null), None),
            (json!(-1), None),
            (json!(u64::from(u32::MAX) + 1), None),
            (json!(1.5), None),
            (json!("123"), None),
        ] {
            let guild: StickerGuild = serde_json::from_value(json!({
                "id": "200", "approximate_presence_count": value, "sticker_count": value
            }))
            .unwrap();
            assert_eq!(guild.approximate_presence_count, expected);
            assert_eq!(guild.sticker_count, expected);
        }
    }

    #[test]
    fn sticker_guild_requires_a_valid_identity() {
        for payload in [json!({}), json!({"id": null}), json!({"id": "bad"})] {
            assert!(serde_json::from_value::<StickerGuild>(payload).is_err());
        }
    }
}
