use super::serde_helpers::*;
use super::*;
use serde::{Deserialize, Deserializer};

fn component_type<'de, D: Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    let kind = u8::deserialize(d)?;
    if matches!(kind, 1..=3 | 5..=16 | 17) {
        Ok(kind)
    } else {
        Err(serde::de::Error::custom("unknown component type"))
    }
}
#[derive(Clone, Deserialize)]
pub struct Component {
    #[serde(rename = "type", deserialize_with = "component_type")]
    pub kind: u8,
    #[serde(default, deserialize_with = "entries")]
    pub components: Vec<Component>,
    #[serde(default, deserialize_with = "default_value")]
    pub accessory: Option<Box<Component>>,
    #[serde(default)]
    pub style: u8,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub emoji: Option<Emoji>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub media: Option<EmbedMedia>,
    #[serde(default)]
    pub file: Option<EmbedMedia>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub spoiler: bool,
    #[serde(default, deserialize_with = "entries")]
    pub items: Vec<GalleryItem>,
    #[serde(default = "default_true")]
    pub divider: bool,
    #[serde(default)]
    pub spacing: u8,
    #[serde(default)]
    pub accent_color: Option<u32>,
    #[serde(default)]
    pub content_inventory_entry: Option<serde_json::Value>,
}

#[derive(Clone, Deserialize)]
pub struct GalleryItem {
    #[serde(default)]
    pub media: EmbedMedia,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub spoiler: bool,
}
