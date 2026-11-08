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
                src: "https://media1.giphy.com/media/v1.abc/sampleGiphyCategoryOne/200.gif".into(),
            },
            GifCategory {
                name: "Adjectives".into(),
                src: "https://media0.giphy.com/media/v1.abc/sampleGiphyCategoryTwo/200.gif".into(),
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
