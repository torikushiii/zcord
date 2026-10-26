use super::*;

fn profile_frame(card: &ProfileCard) -> Option<(u64, &model::CollectibleItem)> {
    let product = card.frame.as_ref()?;
    let item = product.items.iter().find(|item| item.kind == 3)?;
    Some((product.sku_id, item))
}

/// How far the profile frame's art reaches past a card `width` wide, in CSS pixels.
pub(super) fn frame_overflow(card: &ProfileCard, width: f32) -> Option<Edges<f32>> {
    let (_, item) = profile_frame(card)?;
    let scale = width / item.inner_width.unwrap_or(1200).max(1) as f32;
    let horizontal = item.overflow_horizontal.unwrap_or(0) as f32 * scale;
    Some(Edges {
        top: item.overflow_top.unwrap_or(0) as f32 * scale,
        bottom: item.overflow_bottom.unwrap_or(0) as f32 * scale,
        left: horizontal,
        right: horizontal,
    })
}

pub(super) fn framed(
    card: &ProfileCard,
    width: f32,
    body: AnyElement,
    fill: bool,
    cx: &App,
) -> AnyElement {
    let frame = profile_frame(card);
    let layers = |front: bool| -> Vec<AnyElement> {
        let (Some((sku, item)), Some(overflow)) = (frame, frame_overflow(card, width)) else {
            return Vec::new();
        };
        let overflow_x = overflow.left;
        let overflow_top = overflow.top;
        let overflow_bottom = overflow.bottom;
        let frame_width = width + overflow_x * 2.;
        item.layers
            .iter()
            .filter(|layer| (layer.order == "front") == front)
            // The full-profile dialog runs to the window's bottom edge, so Discord drops the
            // bottom staple there and keeps it only on the popout.
            .filter(|layer| !(fill && layer.kind == "staple" && layer.anchor == "bottom"))
            .filter_map(|layer| {
                let spec = ImageSpec::Collectible {
                    url: profile_frame_layer_url(sku, layer.id),
                };
                let path = images::resolve(&spec, theme::image_px(frame_width), cx)?;
                let image = img(path.clone()).w(rpx(frame_width)).flex_none();
                let base = div().absolute().left(rpx(-overflow_x)).w(rpx(frame_width));
                // Border art is a uniform strip of side rails with no fixed height; Discord repeats
                // it down the card and masks it out of the overflow, where the staples sit. Pin one
                // copy to each end of the card and let each fill half, so it fits any card up to
                // twice the art's height without overlapping.
                if layer.kind == "border" {
                    let half = |top: bool| {
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .map(|half| {
                                if top {
                                    half.justify_start()
                                } else {
                                    half.justify_end()
                                }
                            })
                            .child(img(path.clone()).w(rpx(frame_width)).flex_none())
                    };
                    return Some(
                        base.top_0()
                            .bottom_0()
                            .flex()
                            .flex_col()
                            .child(half(true))
                            .child(half(false))
                            .into_any_element(),
                    );
                }
                let base = match (layer.kind.as_str(), layer.anchor.as_str()) {
                    ("rail", anchor) => base
                        .top_0()
                        .bottom_0()
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .map(|rail| match anchor {
                            "top" => rail.justify_start(),
                            "bottom" => rail.justify_end(),
                            _ => rail.justify_center(),
                        }),
                    (_, "bottom") => base.bottom(rpx(-overflow_bottom)),
                    _ => base.top(rpx(-overflow_top)),
                };
                Some(base.child(image).into_any_element())
            })
            .collect()
    };
    div()
        .relative()
        .when(fill, |this| this.h_full())
        .children(layers(false))
        .child(body)
        .children(layers(true))
        .into_any_element()
}
