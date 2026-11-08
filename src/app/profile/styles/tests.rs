use super::encoding::{StyleGifWriter, encode_style_image, prepare_style_image, webp_style_gif};
use super::values::{guild_values, main_values, name_style, normalize_draft, style_patch};
use super::*;
use serde_json::json;

fn inventory_item(sku_id: u64, kind: u8, name: &str) -> OwnedCollectible {
    let product = serde_json::from_value(json!({
        "sku_id": sku_id.to_string(), "type": kind, "name": name, "premium_type": 2,
        "items": [{"sku_id": sku_id.to_string(), "type": kind, "asset": name}]
    }))
    .unwrap();
    OwnedCollectible::from_shop_product(Arc::new(product)).unwrap()
}

#[test]
fn profile_styles_shop_waits_for_owned_and_filters_both_load_orders() {
    let catalog = Arc::new(vec![
        inventory_item(3, 2, "Nameplate"),
        inventory_item(1, 1, "Effect"),
        inventory_item(2, 3, "Frame"),
    ]);
    let owned = Arc::new(vec![catalog[1].clone()]);
    for owned_first in [false, true] {
        let mut editor = StylesEditor::new(StyleScope::Main, true);
        if owned_first {
            editor.owned = Some(owned.clone());
        } else {
            editor.shop_catalog = Some(catalog.clone());
        }
        editor.refresh_shop();
        assert!(editor.shop.is_none());
        editor.owned = Some(owned.clone());
        editor.shop_catalog = Some(catalog.clone());
        editor.refresh_shop();
        let shop = editor.shop.as_ref().unwrap();
        assert_eq!(
            shop.iter().map(|item| item.sku_id).collect::<Vec<_>>(),
            vec![3, 2]
        );
        assert!(shop.iter().all(|item| item.purchased_at.is_none()));
        assert_eq!(editor.shop_of(CollectibleKind::Frame)[0].sku_id, 2);
        assert!(editor.shop_of(CollectibleKind::Effect).is_empty());
        editor.owned = Some(Arc::new(vec![catalog[1].clone(), catalog[0].clone()]));
        editor.refresh_shop();
        assert_eq!(
            editor
                .shop
                .as_ref()
                .unwrap()
                .iter()
                .map(|item| item.sku_id)
                .collect::<Vec<_>>(),
            vec![2]
        );
    }
}

#[test]
fn profile_styles_collectible_lookup_prefers_owned_and_keeps_ownership_separate() {
    let mut editor = StylesEditor::new(StyleScope::Main, true);
    editor.owned = Some(Arc::new(vec![inventory_item(1, 1, "Owned effect")]));
    editor.shop = Some(Arc::new(vec![
        inventory_item(1, 1, "Stale shop effect"),
        inventory_item(2, 3, "Shop frame"),
    ]));
    assert_eq!(
        editor.find_collectible(1).unwrap().product.name,
        "Owned effect"
    );
    assert_eq!(
        editor.find_collectible(2).unwrap().product.name,
        "Shop frame"
    );
    assert!(editor.find_collectible(99).is_none());
    assert!(editor.owned_item(2).is_none());
    assert!(editor.is_owned(1));
    assert!(!editor.is_owned(2));
    assert!(!editor.is_owned(99));
}

#[test]
fn profile_styles_collectible_draft_refuses_shop_skus_in_every_slot() {
    for kind in [
        CollectibleKind::Decoration,
        CollectibleKind::Nameplate,
        CollectibleKind::Effect,
        CollectibleKind::Frame,
    ] {
        let mut editor = StylesEditor::new(StyleScope::Main, true);
        let item = inventory_item(99, 0, "Shop item");
        editor.shop = Some(Arc::new(vec![item.clone()]));
        editor.owned = Some(Arc::new(Vec::new()));
        let field = match kind {
            CollectibleKind::Decoration => &mut editor.draft.decoration,
            CollectibleKind::Nameplate => &mut editor.draft.nameplate,
            CollectibleKind::Effect => &mut editor.draft.effect,
            CollectibleKind::Frame => &mut editor.draft.frame,
        };
        *field = Some(Some(99));
        assert_eq!(
            editor.validate_collectible_draft(),
            Err("You can only save collectibles you own.")
        );
        editor.owned = None;
        assert_eq!(
            editor.validate_collectible_draft(),
            Err("Wait for your collectibles to load before saving.")
        );
        editor.owned = Some(Arc::new(vec![item]));
        assert_eq!(editor.validate_collectible_draft(), Ok(()));
    }
    let mut editor = StylesEditor::new(StyleScope::Main, false);
    editor.draft = StyleDraft {
        decoration: Some(None),
        nameplate: Some(None),
        effect: Some(None),
        frame: Some(None),
        ..Default::default()
    };
    assert_eq!(editor.validate_collectible_draft(), Ok(()));
}

fn profile() -> model::Profile {
    serde_json::from_value(json!({
        "user": {"id": "1", "avatar": "main", "banner": "main-banner", "accent_color": 5,
            "avatar_decoration_data": {"asset": "main-decoration", "sku_id": "10"},
            "collectibles": {"nameplate": {"asset": "main-plate", "sku_id": "11"}},
            "display_name_styles": {"font_id": 14, "effect_id": 3, "colors": [1, 2]}},
        "user_profile": {"theme_colors": [3, 4], "profile_effect": {"sku_id": "12"}, "collectibles": [{"type": 3, "sku_id": "13"}]},
        "guild_member": {"avatar": "server", "banner": "server-banner",
            "avatar_decoration_data": {"asset": "server-decoration", "sku_id": "20"},
            "collectibles": {"nameplate": {"asset": "server-plate", "sku_id": "21"}},
            "display_name_styles": {"font_id": 11, "effect_id": 1, "colors": [2, 3]}},
        "guild_member_profile": {"theme_colors": [5, 6], "profile_effect": {"sku_id": "22"}, "collectibles": [{"type": 3, "sku_id": "23"}]}
    })).unwrap()
}

#[test]
fn profile_styles_saved_values_do_not_inherit_server_overrides() {
    let mut profile = profile();
    let main = main_values(&profile);
    let guild = guild_values(&profile);
    assert_eq!(main.avatar.as_deref(), Some("main"));
    assert_eq!(main.banner.as_deref(), Some("main-banner"));
    assert_eq!(
        (main.decoration, main.nameplate, main.effect, main.frame),
        (Some(10), Some(11), Some(12), Some(13))
    );
    assert_eq!(main.theme_colors, Some([3, 4]));
    assert_eq!(main.accent_color, Some(5));
    assert_eq!(guild.avatar.as_deref(), Some("server"));
    assert_eq!(guild.banner.as_deref(), Some("server-banner"));
    assert_eq!(
        (guild.decoration, guild.nameplate, guild.effect, guild.frame),
        (Some(20), Some(21), Some(22), Some(23))
    );
    assert_eq!(guild.theme_colors, Some([5, 6]));
    profile.guild_member = None;
    profile.guild_member_profile = None;
    assert!(guild_values(&profile) == StyleValues::default());
    assert_eq!(
        name_style(None, None, &[]),
        NameStyle {
            font_id: 11,
            effect_id: 1,
            colors: vec![]
        }
    );
}

#[test]
fn profile_styles_normalize_same_values_and_resets() {
    let saved = main_values(&profile());
    let mut draft = StyleDraft {
        avatar: Some(ImageChange::Recent {
            id: 100,
            storage_hash: "main".into(),
        }),
        decoration: Some(saved.decoration),
        nameplate: Some(saved.nameplate),
        effect: Some(saved.effect),
        frame: Some(saved.frame),
        name_style: Some(saved.name_style.clone()),
        theme_colors: Some(saved.theme_colors),
        accent_color: Some(saved.accent_color),
        ..Default::default()
    };
    normalize_draft(&mut draft, &saved);
    assert!(draft == StyleDraft::default());
    let mut draft = StyleDraft {
        avatar: Some(ImageChange::Remove),
        banner: Some(ImageChange::Remove),
        decoration: Some(None),
        nameplate: Some(None),
        effect: Some(None),
        frame: Some(None),
        name_style: Some(None),
        theme_colors: Some(None),
        accent_color: Some(None),
    };
    normalize_draft(&mut draft, &StyleValues::default());
    assert!(draft == StyleDraft::default());
    let mut draft = StyleDraft {
        name_style: Some(Some(NameStyle {
            font_id: 11,
            effect_id: 0,
            colors: vec![],
        })),
        ..Default::default()
    };
    let saved = StyleValues {
        name_style: Some(NameStyle {
            font_id: 11,
            effect_id: 1,
            colors: vec![],
        }),
        ..Default::default()
    };
    normalize_draft(&mut draft, &saved);
    assert!(draft == StyleDraft::default());
}

#[test]
fn profile_styles_patch_keeps_the_unchanged_effect_or_frame() {
    let saved = main_values(&profile());
    let patch = style_patch(
        &StyleDraft {
            effect: Some(Some(99)),
            ..Default::default()
        },
        &saved,
    );
    assert_eq!(patch.collectibles_sku_ids, Some(vec![99, 13]));
    let patch = style_patch(
        &StyleDraft {
            frame: Some(None),
            ..Default::default()
        },
        &saved,
    );
    assert_eq!(patch.collectibles_sku_ids, Some(vec![12]));
    let patch = style_patch(
        &StyleDraft {
            effect: Some(None),
            frame: Some(None),
            ..Default::default()
        },
        &saved,
    );
    assert_eq!(patch.collectibles_sku_ids, Some(vec![]));
    let patch = style_patch(&StyleDraft::default(), &saved);
    assert_eq!(patch.collectibles_sku_ids, None);
}

#[test]
fn profile_styles_patch_only_changes_the_edited_name_style_component() {
    let saved = main_values(&profile());
    let mut next = saved.name_style.clone().unwrap();
    next.font_id = 16;
    let patch = style_patch(
        &StyleDraft {
            name_style: Some(Some(next)),
            ..Default::default()
        },
        &saved,
    );
    assert_eq!(patch.display_name_font_id, Some(Some(16)));
    assert_eq!(patch.display_name_effect_id, None);
    assert_eq!(patch.display_name_colors, None);
    let patch = style_patch(
        &StyleDraft {
            name_style: Some(None),
            ..Default::default()
        },
        &saved,
    );
    assert_eq!(patch.display_name_font_id, Some(None));
    assert_eq!(patch.display_name_effect_id, Some(None));
    assert_eq!(patch.display_name_colors, Some(None));
}

#[test]
fn profile_styles_preview_is_a_clone_and_resets_inherit_main() {
    let original = Arc::new(profile());
    let mut editor = StylesEditor::new(StyleScope::Guild(7), true);
    editor.saved_profile = Some(original.clone());
    editor.draft = StyleDraft {
        decoration: Some(None),
        nameplate: Some(None),
        name_style: Some(None),
        theme_colors: Some(None),
        effect: Some(None),
        frame: Some(None),
        ..Default::default()
    };
    let preview = editor.preview_profile().unwrap();
    assert_eq!(
        super::super::loading::cosmetic_skus(&preview),
        (Some(12), Some(13))
    );
    let values = guild_values(&preview);
    assert_eq!(
        (
            values.decoration,
            values.nameplate,
            values.effect,
            values.frame
        ),
        (None, None, None, None)
    );
    assert!(values.name_style.is_none() && values.theme_colors.is_none());
    assert_eq!(guild_values(&original).effect, Some(22));
    assert_eq!(guild_values(&original).theme_colors, Some([5, 6]));
}

#[test]
fn profile_styles_previews_always_derive_from_the_fetched_baseline() {
    let original = Arc::new(profile());
    let mut editor = StylesEditor::new(StyleScope::Main, true);
    editor.saved_profile = Some(original.clone());
    editor.draft.effect = Some(Some(99));
    editor.draft.theme_colors = Some(Some([10, 11]));
    let preview = editor.preview_profile().unwrap();
    assert_eq!(main_values(&preview).effect, Some(99));
    assert_eq!(main_values(&preview).theme_colors, Some([10, 11]));
    assert_eq!(
        main_values(editor.saved_profile.as_ref().unwrap()).effect,
        Some(12)
    );
    assert_eq!(
        main_values(editor.saved_profile.as_ref().unwrap()).theme_colors,
        Some([3, 4])
    );
    editor.draft.effect = None;
    let preview = editor.preview_profile().unwrap();
    assert_eq!(main_values(&preview).effect, Some(12));
    assert_eq!(main_values(&preview).theme_colors, Some([10, 11]));
    editor.draft = StyleDraft::default();
    assert!(Arc::ptr_eq(&editor.preview_profile().unwrap(), &original));
    let mut fetched = profile();
    fetched.user_profile.as_mut().unwrap().theme_colors = Some(vec![20, 21]);
    editor.saved_profile = Some(Arc::new(fetched));
    assert_eq!(
        main_values(&editor.preview_profile().unwrap()).theme_colors,
        Some([20, 21])
    );
}

#[test]
fn profile_styles_preview_resolves_owned_assets() {
    let mut editor = StylesEditor::new(StyleScope::Main, true);
    let product: model::CollectibleProduct = serde_json::from_value(json!({
        "sku_id": "30", "name": "Owned", "items": [{"sku_id": "30", "type": 0, "asset": "new-decoration"}]
    })).unwrap();
    editor.owned = Some(Arc::new(vec![OwnedCollectible {
        kind: CollectibleKind::Decoration,
        sku_id: 30,
        product: Arc::new(product),
        purchased_at: None,
    }]));
    editor.draft = StyleDraft {
        decoration: Some(Some(30)),
        effect: Some(Some(31)),
        frame: Some(Some(32)),
        theme_colors: Some(Some([10, 11])),
        ..Default::default()
    };
    let mut preview = profile();
    apply_preview(&mut preview, &editor);
    let decoration = preview
        .user
        .user
        .avatar_decoration_data
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert_eq!(decoration.asset, "new-decoration");
    assert_eq!(decoration.sku_id, Some(30));
    assert_eq!(main_values(&preview).effect, Some(31));
    assert_eq!(main_values(&preview).frame, Some(32));
    assert_eq!(main_values(&preview).theme_colors, Some([10, 11]));
    assert_eq!(editor.owned_of(CollectibleKind::Decoration).len(), 1);
    assert!(editor.owned_of(CollectibleKind::Frame).is_empty());
}

#[test]
fn profile_styles_scope_snapshots_support_arbitrary_guilds() {
    let main = Arc::new(profile());
    let mut card = super::super::ProfileCard::new(
        1,
        None,
        gpui::point(gpui::px(0.), gpui::px(0.)),
        gpui::Anchor::TopLeft,
        super::super::ProfileOrigin::Member,
    );
    card.profile = Some(main.clone());
    card.modal = true;
    assert!(card.switch_to_scope(Some(7)));
    let server_seven = Arc::new(profile());
    card.profile = Some(server_seven.clone());
    assert!(card.switch_to_scope(Some(8)));
    let server_eight = Arc::new(profile());
    card.profile = Some(server_eight.clone());
    assert!(card.switch_to_scope(None));
    assert!(Arc::ptr_eq(card.profile.as_ref().unwrap(), &main));
    assert!(card.switch_to_scope(Some(7)));
    assert!(Arc::ptr_eq(card.profile.as_ref().unwrap(), &server_seven));
    assert!(card.switch_to_scope(Some(8)));
    assert!(Arc::ptr_eq(card.profile.as_ref().unwrap(), &server_eight));
    assert!(card.modal);
    assert!(!card.switch_to_scope(Some(8)));
}

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Png).unwrap();
    bytes.into_inner()
}

fn decode_data_uri(uri: &str) -> image::DynamicImage {
    let bytes = STANDARD.decode(uri.split_once(',').unwrap().1).unwrap();
    image::load_from_memory(&bytes).unwrap()
}

fn animated_webp_bytes() -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], bytes: &[u8]) {
        out.extend_from_slice(tag);
        out.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_le_bytes());
        out.extend_from_slice(bytes);
        if !bytes.len().is_multiple_of(2) {
            out.push(0);
        }
    }
    let mut chunks = Vec::new();
    chunk(&mut chunks, b"VP8X", &[0x12, 0, 0, 0, 31, 0, 0, 15, 0, 0]);
    chunk(&mut chunks, b"ANIM", &[0, 0, 0, 0, 3, 0]);
    for (color, delay) in [([255, 0, 0, 255], 40), ([0, 0, 255, 255], 90)] {
        let mut still = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(32, 16, image::Rgba(color))
            .write_to(&mut still, ImageFormat::WebP)
            .unwrap();
        let mut frame = vec![0, 0, 0, 0, 0, 0, 31, 0, 0, 15, 0, 0, delay, 0, 0, 2];
        frame.extend_from_slice(&still.into_inner()[12..]);
        chunk(&mut chunks, b"ANMF", &frame);
    }
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(&u32::try_from(chunks.len() + 4).unwrap().to_le_bytes());
    bytes.extend_from_slice(b"WEBP");
    bytes.extend_from_slice(&chunks);
    bytes
}

#[test]
fn profile_styles_animated_webp_becomes_gif_without_losing_frames_or_delays() {
    let bytes = animated_webp_bytes();
    for target in [ImageTarget::Avatar, ImageTarget::Banner] {
        let uri = encode_style_image(&bytes, target, true).unwrap();
        assert!(uri.starts_with("data:image/gif;base64,"));
        let gif = STANDARD.decode(uri.split_once(',').unwrap().1).unwrap();
        let decoder = GifDecoder::new(Cursor::new(gif)).unwrap();
        assert!(
            matches!(decoder.loop_count(), image::metadata::LoopCount::Finite(count) if count.get() == 3)
        );
        let frames = decoder.into_frames().collect_frames().unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].delay(), image::Delay::from_numer_denom_ms(40, 1));
        assert_eq!(frames[1].delay(), image::Delay::from_numer_denom_ms(90, 1));
        assert_eq!(frames[0].buffer().get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(frames[1].buffer().get_pixel(0, 0).0, [0, 0, 255, 255]);
    }
    let still = encode_style_image(&bytes, ImageTarget::Avatar, false).unwrap();
    assert!(still.starts_with("data:image/png;base64,"));
    assert_eq!(
        decode_data_uri(&still).to_rgba8().get_pixel(0, 0).0,
        [255, 0, 0, 255]
    );
}

#[test]
fn profile_styles_webp_conversion_rejects_oversized_output_and_invalid_input() {
    let bytes = animated_webp_bytes();
    assert_eq!(
        webp_style_gif(&bytes, 32, None).unwrap_err(),
        "This GIF exceeds the 10 MB upload limit after conversion."
    );
    assert!(webp_style_gif(b"not a webp", STYLE_IMAGE_LIMIT as usize, None).is_err());
    let mut still = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
        .write_to(&mut still, ImageFormat::WebP)
        .unwrap();
    assert!(
        webp_style_gif(&still.into_inner(), STYLE_IMAGE_LIMIT as usize, None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn profile_styles_gif_writer_enforces_the_exclusive_limit() {
    let mut writer = StyleGifWriter {
        bytes: Vec::new(),
        limit: 8,
        overflowed: false,
    };
    writer.write_all(b"1234567").unwrap();
    assert_eq!(
        writer.write_all(b"8").unwrap_err().kind(),
        io::ErrorKind::FileTooLarge
    );
    assert_eq!(writer.bytes, b"1234567");
    assert!(writer.overflowed);
    writer.flush().unwrap();
    assert!(
        encode_style_image(
            &vec![0; STYLE_IMAGE_LIMIT as usize],
            ImageTarget::Avatar,
            true
        )
        .unwrap_err()
        .contains("10 MB")
    );
}

#[test]
fn profile_styles_cached_gif_uses_the_file_upload_pipeline() {
    let path = std::env::temp_dir().join(format!(
        "zcord-style-gif-{}-{:016x}.webp",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::write(&path, animated_webp_bytes()).unwrap();
    let image = prepare_style_image(path.clone(), ImageTarget::Avatar, true);
    std::fs::remove_file(&path).unwrap();
    let ImageChange::Upload {
        path: uploaded_path,
        data_uri,
        ..
    } = image.unwrap()
    else {
        panic!("expected an upload");
    };
    assert_eq!(uploaded_path, path);
    assert!(data_uri.starts_with("data:image/gif;base64,"));
    let bytes = STANDARD
        .decode(data_uri.split_once(',').unwrap().1)
        .unwrap();
    assert_eq!(
        GifDecoder::new(Cursor::new(bytes))
            .unwrap()
            .into_frames()
            .collect_frames()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn profile_styles_images_crop_and_only_resize_down() {
    let avatar = encode_style_image(&png_bytes(1200, 1400), ImageTarget::Avatar, false).unwrap();
    let avatar = decode_data_uri(&avatar);
    assert_eq!((avatar.width(), avatar.height()), (1024, 1024));
    let banner = encode_style_image(&png_bytes(340, 240), ImageTarget::Banner, false).unwrap();
    let banner = decode_data_uri(&banner);
    assert_eq!((banner.width(), banner.height()), (340, 120));
    let banner = encode_style_image(&png_bytes(2550, 900), ImageTarget::Banner, false).unwrap();
    let banner = decode_data_uri(&banner);
    assert!(banner.width() <= 2400 && banner.height() <= 848);
}

#[test]
fn profile_styles_animated_gif_passthrough_requires_nitro() {
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        for color in [[1, 2, 3, 255], [4, 5, 6, 255]] {
            encoder
                .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                    32,
                    16,
                    image::Rgba(color),
                )))
                .unwrap();
        }
    }
    let animated = encode_style_image(&bytes, ImageTarget::Avatar, true).unwrap();
    assert!(animated.starts_with("data:image/gif;base64,"));
    assert_eq!(
        STANDARD
            .decode(animated.split_once(',').unwrap().1)
            .unwrap(),
        bytes
    );
    let still = encode_style_image(&bytes, ImageTarget::Avatar, false).unwrap();
    assert!(still.starts_with("data:image/png;base64,"));
    let still = decode_data_uri(&still);
    assert_eq!((still.width(), still.height()), (16, 16));
}
