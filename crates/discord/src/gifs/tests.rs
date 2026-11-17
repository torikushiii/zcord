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
            gif_src: Some(
                "https://static.klipy.com/ii/7d/c2/f00000000000000000000000000017/gif/1.gif".into()
            ),
            width: 480,
            height: 270,
        }
    );
    assert_eq!(gifs[1].id, "sampleKlipyTwo");
    assert_eq!(gifs[1].title, "Sample GIF Two");
    assert_eq!(gifs[1].gif_src, None);
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

#[test]
fn retired_providers_load_as_klipy() {
    assert_eq!(GifProvider::ALL.map(GifProvider::label), ["Klipy"]);
    assert_eq!(GifProvider::default(), GifProvider::Klipy);
    assert_eq!(
        serde_json::to_string(&GifProvider::Klipy).unwrap(),
        "\"klipy\""
    );
    for name in ["klipy", "tenor", "giphy"] {
        assert_eq!(
            serde_json::from_str::<GifProvider>(&format!("\"{name}\"")).unwrap(),
            GifProvider::Klipy
        );
    }
}
