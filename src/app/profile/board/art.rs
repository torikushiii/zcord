use super::*;

pub(super) fn app_asset(application_id: u64, hash: &str, size: f32, cx: &App) -> Option<Arc<Path>> {
    let spec = ImageSpec::AppImage {
        application_id,
        hash: hash.to_owned(),
        uploaded: false,
        animated: false,
    };
    images::resolve(&spec, theme::image_px(size), cx)
}

pub(super) fn current_art(card: &ProfileCard, application_id: u64, cx: &App) -> Option<Arc<Path>> {
    let art = card
        .applications
        .get(&application_id)
        .and_then(|app| app.icon.as_deref())
        .and_then(|icon| app_asset(application_id, icon, CURRENT_ART, cx));
    art.or_else(|| {
        let game = card.games.get(&application_id)?;
        app_asset(application_id, game.icon_hash.as_deref()?, CURRENT_ART, cx)
    })
}

pub(super) fn activity_art(card: &ProfileCard, entry: &OutboxEntry, cx: &App) -> Option<Arc<Path>> {
    match &entry.kind {
        OutboxKind::PlayedGame { application_id, .. }
        | OutboxKind::LaunchedActivity { application_id, .. } => application_id.and_then(|id| {
            let icon = card.applications.get(&id)?.icon.as_deref()?;
            app_asset(id, icon, ACTIVITY_ART, cx)
        }),
        OutboxKind::Listened { tracks, .. } => tracks
            .first()
            .and_then(|track| track.image_url.as_deref())
            .and_then(|url| media(url, ACTIVITY_ART, cx)),
        OutboxKind::Unknown => None,
    }
}

/// Uploaded widget art, animated when the config marks it so; the still frame stands in while
/// the animation decodes.
pub(super) fn widget_art(
    application_id: u64,
    asset: &WidgetAsset,
    size: f32,
    radius: f32,
    cx: &App,
) -> Option<AnyElement> {
    let still = ImageSpec::AppImage {
        application_id,
        hash: asset.asset_id.to_string(),
        uploaded: true,
        animated: asset.animated,
    };
    let pixels = theme::image_px(size);
    if let Some(frames) = still
        .animated()
        .and_then(|spec| animated::resolve(&spec, pixels, cx))
    {
        return Some(
            animated::image(frames)
                .size_full()
                .rounded(rpx(radius))
                .object_fit(ObjectFit::Cover)
                .id(("widget-art", asset.asset_id))
                .into_any_element(),
        );
    }
    images::resolve(&still, pixels, cx).map(|path| {
        img(path)
            .size_full()
            .rounded(rpx(radius))
            .object_fit(ObjectFit::Cover)
            .into_any_element()
    })
}

pub(super) fn media(url: &str, size: f32, cx: &App) -> Option<Arc<Path>> {
    let spec = ImageSpec::Media {
        url: url.to_owned(),
        width: 0,
        height: 0,
        animated: false,
    };
    images::resolve(&spec, theme::image_px(size), cx)
}

/// Rich-presence art is often a GIF behind Discord's media proxy; Discord plays it, so play it
/// here too, falling back to the still frame while it decodes.
pub(crate) fn presence_art(
    url: &str,
    size: f32,
    radius: f32,
    id: (&'static str, u64),
    cx: &App,
) -> Option<AnyElement> {
    let gif = url
        .split('?')
        .next()
        .is_some_and(|path| path.to_ascii_lowercase().ends_with(".gif"));
    let spec = ImageSpec::Media {
        url: url.to_owned(),
        width: 0,
        height: 0,
        animated: true,
    };
    if gif && let Some(frames) = animated::resolve(&spec, theme::image_px(size), cx) {
        return Some(
            animated::image(frames)
                .size_full()
                .rounded(rpx(radius))
                .object_fit(ObjectFit::Cover)
                .id(id)
                .into_any_element(),
        );
    }
    Some(still_art(media(url, size, cx)?, radius))
}

pub(super) fn still_art(path: Arc<Path>, radius: f32) -> AnyElement {
    img(path)
        .size_full()
        .rounded(rpx(radius))
        .object_fit(ObjectFit::Cover)
        .into_any_element()
}
