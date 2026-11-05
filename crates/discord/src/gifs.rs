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
mod tests {
    use super::*;
    use serde_json::json;

    const KLIPY_GIFS: &str = r#"[
        {
            "id": "sampleKlipyOne",
            "title": "Sample GIF One",
            "url": "https://klipy.com/gifs/sample-one",
            "src": "https://static.klipy.com/ii/7d/c2/f00000000000000000000000000017/webp/1.webp",
            "gif_src": "https://static.klipy.com/ii/7d/c2/f00000000000000000000000000017/gif/1.gif",
            "width": 480,
            "height": 270,
            "preview": "https://static.klipy.com/ii/7d/c2/f00000000000000000000000000017/prev/1.webp"
        },
        {
            "id": "no-source",
            "title": "Broken",
            "url": "https://klipy.com/gifs/broken",
            "width": 100,
            "height": 100
        },
        {
            "id": "sampleKlipyTwo",
            "title": "Sample GIF Two",
            "url": "https://klipy.com/gifs/sample-two",
            "src": "https://static.klipy.com/ii/aa/bb/cat-jump.webp",
            "width": 320,
            "height": 320,
            "preview": "https://static.klipy.com/ii/aa/bb/cat-jump-preview.webp"
        },
        {
            "id": "sampleKlipyOne",
            "title": "Duplicate",
            "url": "https://klipy.com/gifs/duplicate",
            "src": "https://static.klipy.com/ii/cc/duplicate.webp",
            "width": 1,
            "height": 1
        }
    ]"#;

    #[test]
    fn klipy_maps_search_results_and_skips_malformed_or_duplicate_items() {
        let value: Value = serde_json::from_str(KLIPY_GIFS).unwrap();
        let gifs = klipy_gifs(&value);
        assert_eq!(gifs.len(), 2);
        assert_eq!(
            gifs[0],
            Gif {
                id: "sampleKlipyOne".into(),
                title: "Sample GIF One".into(),
                url: "https://klipy.com/gifs/sample-one".into(),
                src: "https://static.klipy.com/ii/7d/c2/f00000000000000000000000000017/webp/1.webp"
                    .into(),
                width: 480,
                height: 270,
            }
        );
        assert_eq!(gifs[1].id, "sampleKlipyTwo");
        assert_eq!(gifs[1].title, "Sample GIF Two");
    }

    #[test]
    fn klipy_trending_categories_use_the_first_gif_as_preview() {
        let value = json!({
            "categories": [
                {"name": "Cats", "src": "https://images-ext-1.discordapp.net/external/cat.webp"},
                {"name": "Reactions", "src": "https://images-ext-1.discordapp.net/external/reaction.webp"},
                {"name": "Broken"}
            ],
            "gifs": [
                {"id": "sampleKlipyOne", "src": "https://static.klipy.com/ii/7d/c2/trending.webp"},
                {"id": "sampleKlipyTwo", "src": "https://static.klipy.com/ii/aa/bb/other.webp"}
            ]
        });
        let categories = klipy_categories(&value);
        assert_eq!(
            categories.categories,
            [
                GifCategory {
                    name: "Cats".into(),
                    src: "https://images-ext-1.discordapp.net/external/cat.webp".into(),
                },
                GifCategory {
                    name: "Reactions".into(),
                    src: "https://images-ext-1.discordapp.net/external/reaction.webp".into(),
                }
            ]
        );
        assert_eq!(
            categories.trending_preview.as_deref(),
            Some("https://static.klipy.com/ii/7d/c2/trending.webp")
        );
    }

    const TENOR_RESULTS: &str = r#"{
        "results": [
            {
                "id": "1191169342046208102",
                "title": "",
                "h1_title": "Sample GIF One",
                "itemurl": "https://tenor.com/view/sample-one-gif-1191169342046208102",
                "media": [{
                    "tinywebm": {
                        "url": "https://media.tenor.com/sampleTenorWebm/sample-one.webm",
                        "dims": [240, 320]
                    },
                    "tinygif": {
                        "url": "https://media.tenor.com/sampleTenorTiny/sample-one.gif",
                        "dims": [165, 221]
                    },
                    "nanogif": {
                        "url": "https://media.tenor.com/sampleTenorNano/sample-one.gif",
                        "dims": [68, 90]
                    },
                    "gif": {
                        "url": "https://media.tenor.com/sampleTenorFull/sample-one.gif",
                        "dims": [374, 498]
                    },
                    "webp": {
                        "url": "https://media.tenor.com/sampleTenorWebp/sample-one.webp",
                        "dims": [374, 498]
                    }
                }]
            },
            {
                "id": "1191169337851904101",
                "title": "",
                "h1_title": "Sample GIF Two",
                "itemurl": "https://tenor.com/view/sample-two-gif-1191169337851904101",
                "media": [{
                    "tinygif": {
                        "url": "https://media.tenor.com/sampleTenorTwoGif/sample-two.gif",
                        "dims": [220, 212]
                    },
                    "mp4": {
                        "url": "https://media.tenor.com/sampleTenorTwoMp4/sample-two.mp4",
                        "dims": [498, 479]
                    }
                }]
            },
            {
                "id": "broken",
                "title": "No media",
                "itemurl": "https://tenor.com/view/broken"
            },
            {
                "id": "1191169342046208102",
                "title": "Duplicate",
                "itemurl": "https://tenor.com/view/duplicate",
                "media": [{"tinygif": {"url": "https://media.tenor.com/duplicate.gif", "dims": [1, 1]}}]
            }
        ],
        "next": "2"
    }"#;

    #[test]
    fn tenor_maps_animated_renditions_and_skips_malformed_or_duplicate_items() {
        let value: Value = serde_json::from_str(TENOR_RESULTS).unwrap();
        let gifs = tenor_gifs(&value);
        assert_eq!(gifs.len(), 2);
        assert_eq!(
            gifs[0],
            Gif {
                id: "1191169342046208102".into(),
                title: "Sample GIF One".into(),
                url: "https://tenor.com/view/sample-one-gif-1191169342046208102".into(),
                src: "https://media.tenor.com/sampleTenorTiny/sample-one.gif".into(),
                width: 165,
                height: 221,
            }
        );
        assert_eq!(
            gifs[1].src,
            "https://media.tenor.com/sampleTenorTwoGif/sample-two.gif"
        );
        assert_eq!((gifs[1].width, gifs[1].height), (220, 212));
    }

    #[test]
    fn tenor_uses_decodable_renditions_only_and_prefers_title() {
        let value = json!({
            "results": [{
                "id": "1",
                "title": "Given title",
                "h1_title": "Fallback",
                "itemurl": "https://tenor.com/view/one",
                "media": [{
                    "tinywebm": {"url": "https://media.tenor.com/a.webm", "dims": [100, 100]},
                    "webp": {"url": "https://media.tenor.com/a.webp", "dims": [400, 300]}
                }]
            }]
        });
        let gifs = tenor_gifs(&value);
        assert_eq!(gifs.len(), 1);
        assert_eq!(gifs[0].title, "Given title");
        assert_eq!(gifs[0].src, "https://media.tenor.com/a.webp");
        assert_eq!((gifs[0].width, gifs[0].height), (400, 300));
        assert!(tenor_gifs(&json!({"results": [{"id": "2", "media": [{}]}]})).is_empty());
    }

    #[test]
    fn tenor_category_mapping() {
        let categories = tenor_categories(&json!({
            "tags": [
                {"searchterm": "embarrassed", "image": "https://media.tenor.com/sampleCategoryOne/dog-staring.gif"},
                {"searchterm": "happy", "image": "https://media.tenor.com/sampleCategoryTwo/happy.gif"},
                {"image": "https://media.tenor.com/missing-searchterm.gif"}
            ]
        }));
        assert_eq!(
            categories.categories,
            [
                GifCategory {
                    name: "embarrassed".into(),
                    src: "https://media.tenor.com/sampleCategoryOne/dog-staring.gif".into(),
                },
                GifCategory {
                    name: "happy".into(),
                    src: "https://media.tenor.com/sampleCategoryTwo/happy.gif".into(),
                }
            ]
        );
        assert_eq!(
            categories.trending_preview.as_deref(),
            Some("https://media.tenor.com/sampleCategoryOne/dog-staring.gif")
        );
    }

    const GIPHY_RESULTS: &str = r#"{
        "data": [
            {
                "type": "gif",
                "id": "sampleGiphyOne",
                "title": "Sample GIF One",
                "url": "https://giphy.com/gifs/sample-one-sampleGiphyOne",
                "images": {
                    "fixed_height": {
                        "url": "https://media4.giphy.com/media/v1.sampleTrackingPath/sampleGiphyOne/200.gif",
                        "webp": "https://media4.giphy.com/media/v1.sampleTrackingPath/sampleGiphyOne/200.webp",
                        "width": "200",
                        "height": "200"
                    },
                    "original": {
                        "url": "https://media4.giphy.com/media/v1.sampleTrackingPath/sampleGiphyOne/giphy.gif"
                    }
                }
            },
            {
                "type": "gif",
                "id": "sampleGiphyTwo",
                "title": "Sample GIF Two",
                "url": "https://giphy.com/gifs/sample-two-sampleGiphyTwo",
                "images": {
                    "fixed_height": {
                        "url": "https://media1.giphy.com/media/v1.sampleTrackingPath/sampleGiphyTwo/200.gif",
                        "webp": "https://media1.giphy.com/media/v1.sampleTrackingPath/sampleGiphyTwo/200.webp",
                        "width": "200",
                        "height": "200"
                    },
                    "original": {
                        "url": "https://media1.giphy.com/media/v1.sampleTrackingPath/sampleGiphyTwo/giphy.gif"
                    }
                }
            },
            {
                "id": "broken",
                "title": "No images",
                "url": "https://giphy.com/gifs/broken"
            }
        ]
    }"#;

    #[test]
    fn giphy_maps_fixed_height_webp_and_skips_malformed_items() {
        let value: Value = serde_json::from_str(GIPHY_RESULTS).unwrap();
        let gifs = giphy_gifs(&value);
        assert_eq!(gifs.len(), 2);
        assert_eq!(gifs[0].id, "sampleGiphyOne");
        assert_eq!(gifs[0].title, "Sample GIF One");
        assert_eq!(
            gifs[0].url,
            "https://giphy.com/gifs/sample-one-sampleGiphyOne"
        );
        assert!(gifs[0].src.starts_with("https://media4.giphy.com/"));
        assert!(gifs[0].src.ends_with("/sampleGiphyOne/200.webp"));
        assert_eq!((gifs[0].width, gifs[0].height), (200, 200));
        assert!(gifs[1].src.starts_with("https://media1.giphy.com/"));
        assert!(gifs[1].src.ends_with("/sampleGiphyTwo/200.webp"));

        let fallback = giphy_gifs(&json!({"data": [{
            "id": "fallback",
            "url": "https://giphy.com/gifs/fallback",
            "images": {"fixed_height": {
                "url": "https://media0.giphy.com/fallback/200.gif",
                "width": "320",
                "height": "240"
            }}
        }]}));
        assert_eq!(fallback[0].src, "https://media0.giphy.com/fallback/200.gif");
        assert_eq!((fallback[0].width, fallback[0].height), (320, 240));
    }

    #[test]
    fn giphy_category_mapping() {
        let categories = giphy_categories(&json!({
            "data": [
                {"name": "Actions", "gif": {"images": {"fixed_height": {
                    "url": "https://media1.giphy.com/media/v1.abc/sampleGiphyCategoryOne/200.gif"
                }}}},
                {"name": "Adjectives", "gif": {"images": {"fixed_height": {
                    "url": "https://media0.giphy.com/media/v1.abc/sampleGiphyCategoryTwo/200.gif"
                }}}},
                {"name": "Broken"}
            ]
        }));
        assert_eq!(
            categories.categories,
            [
                GifCategory {
                    name: "Actions".into(),
                    src: "https://media1.giphy.com/media/v1.abc/sampleGiphyCategoryOne/200.gif"
                        .into(),
                },
                GifCategory {
                    name: "Adjectives".into(),
                    src: "https://media0.giphy.com/media/v1.abc/sampleGiphyCategoryTwo/200.gif"
                        .into(),
                }
            ]
        );
        assert_eq!(
            categories.trending_preview.as_deref(),
            Some("https://media1.giphy.com/media/v1.abc/sampleGiphyCategoryOne/200.gif")
        );
    }

    #[test]
    fn provider_labels_and_settings_names() {
        assert_eq!(
            GifProvider::ALL.map(GifProvider::label),
            ["Klipy", "Tenor", "Giphy"]
        );
        assert_eq!(GifProvider::default(), GifProvider::Klipy);
        for (name, provider) in [
            ("klipy", GifProvider::Klipy),
            ("tenor", GifProvider::Tenor),
            ("giphy", GifProvider::Giphy),
        ] {
            assert_eq!(
                serde_json::to_string(&provider).unwrap(),
                format!("\"{name}\"")
            );
            assert_eq!(
                serde_json::from_str::<GifProvider>(&format!("\"{name}\"")).unwrap(),
                provider
            );
        }
    }
}
