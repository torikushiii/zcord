//! GIF picker providers.
//!
//! Klipy is served through Discord's own API and `rest::Rest`. Tenor and Giphy are third
//! parties, so their requests go through a bare client that carries no Discord credentials.
use super::{Error, properties};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

const TENOR_KEY: &str = "3Z0688EVWYKH";
const TENOR_BASE: &str = "https://api.tenor.com/v1";
const GIPHY_KEY: &str = "Gc7131jiJuvI7IdN0HZ1D7nh0ow5BU6g";
const GIPHY_BASE: &str = "https://api.giphy.com/v1";
const PAGE_LIMIT: &str = "50";

/// Tenor's animated renditions the image pipeline can decode, smallest first. It also serves
/// mp4/webm, which it cannot animate.
const TENOR_FORMATS: [&str; 4] = ["tinygif", "nanogif", "webp", "gif"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GifProvider {
    #[default]
    Klipy,
    Tenor,
    Giphy,
}

impl GifProvider {
    pub const ALL: [GifProvider; 3] = [GifProvider::Klipy, GifProvider::Tenor, GifProvider::Giphy];

    pub fn label(self) -> &'static str {
        match self {
            Self::Klipy => "Klipy",
            Self::Tenor => "Tenor",
            Self::Giphy => "Giphy",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gif {
    pub id: String,
    pub title: String,
    /// Page URL, sent as the message content.
    pub url: String,
    /// Animated gif/webp rendition shown in the picker grid.
    pub src: String,
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

fn categories(
    value: Option<&Value>,
    category: impl Fn(&Value) -> Option<GifCategory>,
    preview: impl Fn(&[GifCategory]) -> Option<String>,
) -> GifCategories {
    let categories: Vec<_> = value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(&category).collect())
        .unwrap_or_default();
    let trending_preview = preview(&categories);
    GifCategories {
        categories,
        trending_preview,
    }
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
        width: dimension(value, "width")?,
        height: dimension(value, "height")?,
    })
}

pub(super) fn klipy_gifs(value: &Value) -> Vec<Gif> {
    let items = value.as_array().map(Vec::as_slice).unwrap_or_default();
    dedupe(items.iter().filter_map(klipy_gif))
}

pub(super) fn klipy_categories(value: &Value) -> GifCategories {
    categories(
        value.get("categories"),
        |item| {
            Some(GifCategory {
                name: text(item, "name")?.to_owned(),
                src: text(item, "src")?.to_owned(),
            })
        },
        |_| {
            value
                .get("gifs")
                .and_then(Value::as_array)
                .and_then(|gifs| gifs.first())
                .and_then(|gif| text(gif, "src"))
                .map(str::to_owned)
        },
    )
}

fn dims(rendition: &Value) -> Option<(u32, u32)> {
    let dims = rendition.get("dims")?.as_array()?;
    let width = u32::try_from(dims.first()?.as_u64()?).ok()?;
    let height = u32::try_from(dims.get(1)?.as_u64()?).ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

fn tenor_rendition(media: &[Value]) -> Option<(String, u32, u32)> {
    for format in TENOR_FORMATS {
        for item in media {
            let Some(rendition) = item.get(format) else {
                continue;
            };
            let (Some(url), Some((width, height))) = (text(rendition, "url"), dims(rendition))
            else {
                continue;
            };
            return Some((url.to_owned(), width, height));
        }
    }
    None
}

fn tenor_gif(value: &Value) -> Option<Gif> {
    let media = value.get("media")?.as_array()?;
    let (src, width, height) = tenor_rendition(media)?;
    Some(Gif {
        id: text(value, "id")?.to_owned(),
        title: text(value, "title")
            .or_else(|| text(value, "h1_title"))
            .unwrap_or_default()
            .to_owned(),
        url: text(value, "itemurl")?.to_owned(),
        src,
        width,
        height,
    })
}

pub(super) fn tenor_gifs(value: &Value) -> Vec<Gif> {
    let items = value
        .get("results")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    dedupe(items.iter().filter_map(tenor_gif))
}

pub(super) fn tenor_categories(value: &Value) -> GifCategories {
    categories(
        value.get("tags"),
        |item| {
            Some(GifCategory {
                name: text(item, "searchterm")?.to_owned(),
                src: text(item, "image")?.to_owned(),
            })
        },
        |categories| categories.first().map(|category| category.src.clone()),
    )
}

fn giphy_gif(value: &Value) -> Option<Gif> {
    let fixed = value.get("images")?.get("fixed_height")?;
    Some(Gif {
        id: text(value, "id")?.to_owned(),
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        url: text(value, "url")?.to_owned(),
        src: text(fixed, "webp")
            .or_else(|| text(fixed, "url"))?
            .to_owned(),
        width: dimension(fixed, "width")?,
        height: dimension(fixed, "height")?,
    })
}

pub(super) fn giphy_gifs(value: &Value) -> Vec<Gif> {
    let items = value
        .get("data")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    dedupe(items.iter().filter_map(giphy_gif))
}

pub(super) fn giphy_categories(value: &Value) -> GifCategories {
    categories(
        value.get("data"),
        |item| {
            Some(GifCategory {
                name: text(item, "name")?.to_owned(),
                src: item
                    .get("gif")?
                    .get("images")?
                    .get("fixed_height")
                    .and_then(|fixed| text(fixed, "url"))?
                    .to_owned(),
            })
        },
        |categories| categories.first().map(|category| category.src.clone()),
    )
}

fn giphy_lang() -> String {
    let locale = properties::LOCALE.to_ascii_lowercase();
    if locale.starts_with("zh") {
        if locale.contains("tw") || locale.contains("hant") {
            "zh-TW".to_owned()
        } else {
            "zh-CN".to_owned()
        }
    } else {
        locale.split('-').next().unwrap_or("en").to_owned()
    }
}

fn tenor_locale() -> String {
    properties::LOCALE.to_ascii_lowercase().replace('-', "_")
}

async fn get_json(client: &Client, url: &str, query: &[(&str, &str)]) -> Result<Value, Error> {
    let response = client
        .get(url)
        .query(query)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    let status = response.status();
    if !status.is_success() {
        return Err(Error::Http(status.as_u16()));
    }
    response.json().await.map_err(|_| Error::Decode)
}

pub(super) async fn tenor_search(client: &Client, query: &str) -> Result<Vec<Gif>, Error> {
    let locale = tenor_locale();
    let value = get_json(
        client,
        &format!("{TENOR_BASE}/search"),
        &[
            ("key", TENOR_KEY),
            ("locale", &locale),
            ("q", query),
            ("limit", PAGE_LIMIT),
        ],
    )
    .await?;
    Ok(tenor_gifs(&value))
}

pub(super) async fn tenor_trending(client: &Client) -> Result<Vec<Gif>, Error> {
    let locale = tenor_locale();
    let value = get_json(
        client,
        &format!("{TENOR_BASE}/trending"),
        &[
            ("key", TENOR_KEY),
            ("locale", &locale),
            ("limit", PAGE_LIMIT),
        ],
    )
    .await?;
    Ok(tenor_gifs(&value))
}

pub(super) async fn tenor_categories_request(client: &Client) -> Result<GifCategories, Error> {
    let locale = tenor_locale();
    let value = get_json(
        client,
        &format!("{TENOR_BASE}/categories"),
        &[
            ("key", TENOR_KEY),
            ("locale", &locale),
            ("type", "featured"),
        ],
    )
    .await?;
    Ok(tenor_categories(&value))
}

pub(super) async fn giphy_search(client: &Client, query: &str) -> Result<Vec<Gif>, Error> {
    let lang = giphy_lang();
    let value = get_json(
        client,
        &format!("{GIPHY_BASE}/gifs/search"),
        &[
            ("api_key", GIPHY_KEY),
            ("lang", &lang),
            ("q", query),
            ("limit", PAGE_LIMIT),
        ],
    )
    .await?;
    Ok(giphy_gifs(&value))
}

pub(super) async fn giphy_trending(client: &Client) -> Result<Vec<Gif>, Error> {
    let lang = giphy_lang();
    let value = get_json(
        client,
        &format!("{GIPHY_BASE}/gifs/trending"),
        &[
            ("api_key", GIPHY_KEY),
            ("lang", &lang),
            ("limit", PAGE_LIMIT),
        ],
    )
    .await?;
    Ok(giphy_gifs(&value))
}

pub(super) async fn giphy_categories_request(client: &Client) -> Result<GifCategories, Error> {
    let lang = giphy_lang();
    let value = get_json(
        client,
        &format!("{GIPHY_BASE}/gifs/categories"),
        &[("api_key", GIPHY_KEY), ("lang", &lang)],
    )
    .await?;
    Ok(giphy_categories(&value))
}

#[cfg(test)]
mod tests;
