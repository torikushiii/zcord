use super::media::{mosaic, spoiler_cover, spoiler_media};
use super::reactions::emoji_element;
use super::*;

const CONTAINER_WIDTH: f32 = 600.;
const SECTION_THUMBNAIL: f32 = 85.;
const SPOILER_SLOT: u64 = 1 << 21;

pub fn render_components(
    app: &DiscordApp,
    message_id: u64,
    components: &[Component],
    cx: &App,
) -> Option<AnyElement> {
    if components.is_empty() {
        return None;
    }
    let mut counter = 0;
    Some(
        div()
            .flex()
            .flex_col()
            .gap(rpx(8.))
            .mt_1()
            .w_full()
            .children(
                components.iter().map(|component| {
                    render_component(app, message_id, component, &mut counter, cx)
                }),
            )
            .into_any_element(),
    )
}

fn render_component(
    app: &DiscordApp,
    message_id: u64,
    component: &Component,
    counter: &mut u64,
    cx: &App,
) -> AnyElement {
    *counter += 1;
    let slot = *counter;
    match component {
        Component::Row(children) => div()
            .flex()
            .flex_wrap()
            .gap(rpx(8.))
            .children(
                children
                    .iter()
                    .map(|child| render_component(app, message_id, child, counter, cx)),
            )
            .into_any_element(),
        Component::Button {
            style,
            label,
            emoji,
            url,
            disabled,
        } => button(
            part_key(message_id, COMPONENT_SLOT + slot),
            *style,
            label.as_deref(),
            emoji.as_ref(),
            url.as_ref(),
            *disabled,
            cx,
        ),
        Component::Select {
            placeholder,
            disabled,
        } => div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .max_w(rpx(400.))
            .h(rpx(40.))
            .px(rpx(12.))
            .rounded(rpx(8.))
            .border_1()
            .border_color(rgba(theme::border()))
            .bg(rgb(theme::bg()))
            .when(*disabled, |this| this.opacity(0.5))
            .child(
                div()
                    .truncate()
                    .text_size(rpx(14.))
                    .text_color(rgb(theme::muted()))
                    .child(
                        placeholder
                            .clone()
                            .unwrap_or_else(|| "Make a selection".into()),
                    ),
            )
            .child(theme::icon("chevron-down", 18., theme::muted()))
            .into_any_element(),
        Component::Section {
            components,
            accessory,
        } => div()
            .flex()
            .items_start()
            .gap(rpx(16.))
            .w_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(rpx(8.))
                    .children(
                        components
                            .iter()
                            .map(|child| render_component(app, message_id, child, counter, cx)),
                    ),
            )
            .children(accessory.as_deref().map(|accessory| match accessory {
                Component::Thumbnail { media, spoiler, .. } => {
                    *counter += 1;
                    let key = part_key(message_id, SPOILER_SLOT + *counter);
                    let (width, height) = fit(media, (SECTION_THUMBNAIL, SECTION_THUMBNAIL));
                    let (width, height) = if media.width == 0 {
                        (SECTION_THUMBNAIL, SECTION_THUMBNAIL)
                    } else {
                        (width, height)
                    };
                    spoiler_media(app, key, *spoiler, media, width, height, 8., cx)
                        .into_any_element()
                }
                other => render_component(app, message_id, other, counter, cx),
            }))
            .into_any_element(),
        Component::Text(content) => markdown(
            app,
            part_key(message_id, COMPONENT_SLOT + slot),
            content,
            14.,
            20.,
            cx,
        ),
        Component::Thumbnail { media, spoiler, .. } => {
            let (width, height) = fit(media, (SECTION_THUMBNAIL, SECTION_THUMBNAIL));
            spoiler_media(
                app,
                part_key(message_id, SPOILER_SLOT + slot),
                *spoiler,
                media,
                width,
                height,
                8.,
                cx,
            )
            .into_any_element()
        }
        Component::Gallery(items) => gallery(app, message_id, items, counter, cx),
        Component::File {
            name,
            size,
            spoiler,
        } => div()
            .flex()
            .items_center()
            .gap_3()
            .max_w(rpx(432.))
            .p_3()
            .rounded(rpx(8.))
            .bg(rgb(theme::bg()))
            .border_1()
            .border_color(rgba(theme::border()))
            .when(*spoiler, |this| this.opacity(0.6))
            .child(theme::icon("file", 32., theme::muted()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_color(rgb(theme::link()))
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .text_size(rpx(12.))
                            .text_color(rgb(theme::faint()))
                            .child(file_size(*size)),
                    ),
            )
            .into_any_element(),
        Component::Separator { divider, large } => {
            let space = if *large { 16. } else { 8. };
            div()
                .w_full()
                .py(rpx(space / 2.))
                .when(*divider, |this| {
                    this.child(div().w_full().h(px(1.)).bg(rgba(theme::border())))
                })
                .into_any_element()
        }
        Component::Container {
            accent,
            spoiler,
            components,
        } => {
            let key = part_key(message_id, SPOILER_SLOT + slot);
            let element = component_container(
                *accent,
                components
                    .iter()
                    .map(|child| render_component(app, message_id, child, counter, cx)),
            );
            spoiler_cover(app, key, *spoiler, element)
        }
        Component::ActivityEntry(entry) => {
            crate::app::members::member_list::activity::dm_card(app, entry, cx)
        }
    }
}

pub(super) fn component_container(
    accent: Option<u32>,
    children: impl IntoIterator<Item = AnyElement>,
) -> Div {
    // Resolve the constrained column width before measuring long wrapped text.
    // Nested flex rows can retain a height measured at the wider message width.
    div()
        .relative()
        .grid()
        .grid_cols(1)
        .flex_shrink_0()
        .w_full()
        .max_w(rpx(CONTAINER_WIDTH))
        .gap(rpx(8.))
        .p(rpx(16.))
        .when(accent.is_some(), |this| this.pl(rpx(20.)))
        .rounded(rpx(8.))
        .overflow_hidden()
        .border_1()
        .border_color(rgba(theme::border()))
        .bg(rgb(theme::elevated()))
        .children(accent.map(|color| {
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(rpx(4.))
                .bg(rgb(color))
        }))
        .children(children)
}

pub(super) fn single_gallery_frame(
    element: Stateful<Div>,
    width: f32,
    height: f32,
) -> Stateful<Div> {
    element.max_w_full().h_auto().aspect_ratio(width / height)
}

fn gallery(
    app: &DiscordApp,
    message_id: u64,
    items: &[GalleryItem],
    counter: &mut u64,
    cx: &App,
) -> AnyElement {
    let mut keyed = Vec::with_capacity(items.len());
    for item in items {
        *counter += 1;
        keyed.push((part_key(message_id, SPOILER_SLOT + *counter), item));
    }
    if let [(key, item)] = keyed.as_slice() {
        let (width, height) = fit(&item.media, PLAIN_MEDIA);
        return single_gallery_frame(
            spoiler_media(app, *key, item.spoiler, &item.media, width, height, 8., cx),
            width,
            height,
        )
        .into_any_element();
    }
    let Some((_, lead)) = keyed.first() else {
        return div().into_any_element();
    };
    let lead = (lead.media.width, lead.media.height);
    mosaic(keyed.len(), PLAIN_MEDIA.0, lead, |slot, w, h| {
        let (key, item) = keyed[slot];
        spoiler_media(app, key, item.spoiler, &item.media, w, h, 4., cx).into_any_element()
    })
    .into_any_element()
}

fn button(
    key: u64,
    style: u8,
    label: Option<&str>,
    emoji: Option<&ReactionEmoji>,
    url: Option<&String>,
    disabled: bool,
    cx: &App,
) -> AnyElement {
    let (background, border) = match style {
        1 | 6 => (theme::BLURPLE, theme::BLURPLE),
        3 => (0x00863A, 0x00863A),
        4 => (0xD22D39, 0xD22D39),
        _ => (theme::button_bg(), theme::button_border()),
    };
    let url = url.filter(|_| style == 5 && !disabled).cloned();
    div()
        .id(("component-button", key))
        .flex()
        .items_center()
        .gap(rpx(6.))
        .h(rpx(32.))
        .px(rpx(16.))
        .rounded(rpx(8.))
        .border_1()
        .border_color(rgb(border))
        .bg(rgb(background))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(if matches!(style, 1 | 3 | 4 | 6) {
            white()
        } else {
            rgb(theme::bright()).into()
        })
        .when(disabled, |this| this.opacity(0.5))
        .children(emoji.map(|emoji| emoji_element(emoji, 20., cx)))
        .children(label.map(|label| wrapped_label(("component-label", key), label)))
        .when(style == 5, |this| {
            this.child(theme::icon("external-link", 16., theme::bright()))
        })
        .when_some(url, |this, url| {
            this.cursor_pointer()
                .on_click(move |_, _, cx| cx.open_url(&url))
        })
        .into_any_element()
}
