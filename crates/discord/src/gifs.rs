use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

/// Saved settings may still name the retired `tenor` or `giphy` providers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GifProvider {
    #[default]
    #[serde(other)]
    Klipy,
}

impl GifProvider {
    pub const ALL: [GifProvider; 1] = [GifProvider::Klipy];

    pub fn label(self) -> &'static str {
        match self {
            Self::Klipy => "Klipy",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gif {
    pub id: String,
    pub title: String,
    pub url: String,
    pub src: String,
    pub gif_src: Option<String>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GifCategory {
    pub name: String,
    pub src: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GifCategories {
    pub categories: Vec<GifCategory>,
    pub trending_preview: Option<String>,
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

fn dimension(value: &Value, key: &str) -> Option<u32> {
    let number = match value.get(key)? {
        Value::String(text) => text.parse::<u32>().ok()?,
        value => u32::try_from(value.as_u64()?).ok()?,
    };
    (number > 0).then_some(number)
}

fn dedupe(gifs: impl IntoIterator<Item = Gif>) -> Vec<Gif> {
    let mut seen = HashSet::new();
    gifs.into_iter()
        .filter(|gif| seen.insert(gif.id.clone()))
        .collect()
}

fn klipy_gif(value: &Value) -> Option<Gif> {
    Some(Gif {
        id: text(value, "id")?.to_owned(),
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        url: text(value, "url")?.to_owned(),
        src: text(value, "src")?.to_owned(),
        gif_src: text(value, "gif_src").map(str::to_owned),
        width: dimension(value, "width")?,
        height: dimension(value, "height")?,
    })
}

pub(super) fn klipy_gifs(value: &Value) -> Vec<Gif> {
    let items = value.as_array().map(Vec::as_slice).unwrap_or_default();
    dedupe(items.iter().filter_map(klipy_gif))
}

pub(super) fn klipy_categories(value: &Value) -> GifCategories {
    let categories = value
        .get("categories")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(GifCategory {
                        name: text(item, "name")?.to_owned(),
                        src: text(item, "src")?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let trending_preview = value
        .get("gifs")
        .and_then(Value::as_array)
        .and_then(|gifs| gifs.first())
        .and_then(|gif| text(gif, "src"))
        .map(str::to_owned);
    GifCategories {
        categories,
        trending_preview,
    }
}

#[cfg(test)]
mod tests;
