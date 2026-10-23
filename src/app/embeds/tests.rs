use std::{borrow::Cow, cell::RefCell, rc::Rc};

use gpui::{
    Context, FontWeight, IntoElement, ListAlignment, ListState, Render, TestAppContext, Window,
    div, list, prelude::*, px,
};
use gpui_component::ElementExt;

use super::{
    EMBED_HORIZONTAL_INSET, EMBED_MEDIA, EMBED_WIDTH, component_container, description_text,
    embed_card, fit, single_gallery_frame,
};
use crate::{
    app::rich_text::{Piece, PieceStyle, rich_text},
    store::Media,
    theme::rpx,
};

#[test]
fn only_orphaned_line_start_selectors_are_removed() {
    let source = "Beach skin! \u{1F3D6}\u{FE0F}\n\u{FE00}\u{FE00}\n\u{FE00}\u{FE00}[#ZZZ](https://example.com)\n  indented\n**bold**";
    assert_eq!(
        description_text(source),
        "Beach skin! \u{1F3D6}\u{FE0F}\n\n[#ZZZ](https://example.com)\n  indented\n**bold**"
    );
    for source in [
        "",
        "normal\n  indentation",
        "\u{1F3D6}\u{FE0F}",
        "\u{4E00}\u{FE00}",
    ] {
        assert!(matches!(description_text(source), Cow::Borrowed(_)));
    }
}

#[test]
fn media_dimensions_keep_their_existing_limits_and_aspect_ratio() {
    let media = |width, height| Media {
        url: "https://cdn.discordapp.com/attachments/1/2/image.png".into(),
        source: String::new(),
        width,
        height,
        animated: false,
    };
    assert_eq!(fit(&media(1920, 1080), EMBED_MEDIA), (400., 225.));
    assert_eq!(fit(&media(1080, 1920), EMBED_MEDIA), (169., 300.));
    assert_eq!(fit(&media(120, 80), EMBED_MEDIA), (120., 80.));
    assert_eq!(fit(&media(0, 0), EMBED_MEDIA), EMBED_MEDIA);
}

struct EmbedLayout {
    media_width: Option<f32>,
    description: String,
    side_thumbnail: bool,
    weight: Rc<RefCell<Option<FontWeight>>>,
}

impl Render for EmbedLayout {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let weight = self.weight.clone();
        let body = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .debug_selector(|| "description".into())
            .on_prepaint(move |_, window, _| {
                *weight.borrow_mut() = Some(window.text_style().font_weight)
            })
            .child(
                rich_text(
                    "description",
                    vec![Piece::text(
                        description_text(&self.description).into_owned(),
                        PieceStyle::default(),
                    )],
                )
                .line_height(rpx(18.)),
            );
        let content =
            div()
                .flex()
                .gap(rpx(16.))
                .w_full()
                .child(body)
                .when(self.side_thumbnail, |this| {
                    this.child(
                        div()
                            .flex_none()
                            .size(rpx(80.))
                            .debug_selector(|| "thumbnail".into()),
                    )
                });
        div().size_full().font_weight(FontWeight::BOLD).child(
            embed_card(None, self.media_width)
                .debug_selector(|| "card".into())
                .child(content)
                .children(self.media_width.map(|width| {
                    div()
                        .flex_none()
                        .w(rpx(width))
                        .h(rpx(225.))
                        .debug_selector(|| "media".into())
                })),
        )
    }
}

#[gpui::test]
fn text_wraps_to_the_media_width_at_each_zoom(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let weight = Rc::new(RefCell::new(None));
    let (_, cx) = cx.add_window_view(|_, _| EmbedLayout {
        media_width: Some(400.),
        description: "Beach event is almost over... Don't forget to claim your free Lucy skin!"
            .into(),
        side_thumbnail: false,
        weight: weight.clone(),
    });
    for zoom in [1., 1.25, 1.5] {
        cx.update(|window, cx| {
            window.set_rem_size(px(16. * zoom));
            window.refresh();
            let _ = window.draw(cx);
        });
        let card = cx.debug_bounds("card").unwrap();
        let description = cx.debug_bounds("description").unwrap();
        let media = cx.debug_bounds("media").unwrap();
        assert!((card.size.width - px((400. + EMBED_HORIZONTAL_INSET) * zoom)).abs() <= px(1.));
        assert!((description.size.width - media.size.width).abs() <= px(1.));
        assert!((description.left() - media.left()).abs() <= px(1.));
        assert!(description.size.height >= px(36. * zoom - 1.));
        assert!((media.size.width - px(400. * zoom)).abs() <= px(1.));
        assert_eq!(*weight.borrow(), Some(FontWeight::NORMAL));
    }
}

#[gpui::test]
fn text_only_and_side_thumbnail_cards_keep_the_existing_width_limit(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| EmbedLayout {
        media_width: None,
        description: "Short description".into(),
        side_thumbnail: false,
        weight: Rc::new(RefCell::new(None)),
    });
    cx.update(|window, cx| {
        window.set_rem_size(px(16.));
        let _ = window.draw(cx);
    });
    let compact = cx.debug_bounds("card").unwrap();
    assert!(compact.size.width <= px(EMBED_WIDTH));
    view.update(cx, |view, cx| {
        view.side_thumbnail = true;
        cx.notify();
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let card = cx.debug_bounds("card").unwrap();
    let thumbnail = cx.debug_bounds("thumbnail").unwrap();
    assert!(card.size.width <= px(EMBED_WIDTH));
    assert_eq!(thumbnail.size.width, px(80.));
    assert!(thumbnail.right() < card.right());
}

struct LongComponentEmbed {
    width: f32,
    list: ListState,
}

impl Render for LongComponentEmbed {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(self.width))
            .h(px(600.))
            .child(gpui_base::TextSelectionLayer)
            .child(list(self.list.clone(), |_, _, _| long_component_row()).size_full())
    }
}

fn long_component_row() -> gpui::AnyElement {
    let text = (0..12)
        .map(|_| {
            "A long post should wrap to the card width without clipping the image below it. "
                .repeat(5)
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let paragraph = rich_text("long-post", vec![Piece::text(text, PieceStyle::default())])
        .selectable()
        .line_height(rpx(20.));
    let text = div()
        .w_full()
        .min_w_0()
        .debug_selector(|| "component-text".into())
        .text_size(rpx(14.))
        .line_height(rpx(20.))
        .child(div().flex().flex_col().w_full().min_w_0().child(paragraph));
    let image = single_gallery_frame(
        div()
            .id("gallery-image")
            .flex_none()
            .w(rpx(416.))
            .h(rpx(350.))
            .debug_selector(|| "gallery-image".into()),
        416.,
        350.,
    );
    let card = component_container(
        Some(0xFF4500),
        [text.into_any_element(), image.into_any_element()],
    )
    .debug_selector(|| "component-card".into());
    div()
        .flex()
        .flex_col()
        .w_full()
        .child(
            div()
                .flex()
                .w_full()
                .child(div().w(rpx(72.)).flex_none())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(rpx(8.))
                                .mt_1()
                                .w_full()
                                .child(card),
                        )
                        .child(div().h(rpx(24.)).debug_selector(|| "reactions".into())),
                ),
        )
        .into_any_element()
}

#[gpui::test]
fn long_component_posts_reserve_the_full_gallery_height(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| LongComponentEmbed {
        width: 700.,
        list: ListState::new(1, ListAlignment::Top, px(100.)),
    });
    for zoom in [1., 1.125, 1.25, 1.5] {
        for width in [700., 480., 320.] {
            view.update(cx, |view, cx| {
                view.width = width;
                view.list.reset(1);
                cx.notify();
            });
            cx.update(|window, cx| {
                window.set_rem_size(px(16. * zoom));
                window.refresh();
                crate::app::rich_text::metrics::SHAPES.set(0);
                crate::app::rich_text::metrics::LAYOUTS.set(0);
                crate::app::rich_text::metrics::PAINTS.set(0);
                let _ = window.draw(cx);
            });
            let card = cx.debug_bounds("component-card").unwrap();
            let image = cx.debug_bounds("gallery-image").unwrap();
            assert!(image.size.width <= px(416. * zoom));
            assert!((image.size.height - image.size.width * (350. / 416.)).abs() <= px(1.));
            assert!(image.right() < card.right());
            assert!(
                image.bottom() <= card.bottom(),
                "zoom={zoom} width={width}: image={image:?} card={card:?}"
            );
            assert!(cx.debug_bounds("reactions").unwrap().top() >= card.bottom());
            assert!(crate::app::rich_text::metrics::SHAPES.get() <= 1800);
            assert!(crate::app::rich_text::metrics::LAYOUTS.get() <= 6);
            assert!(crate::app::rich_text::metrics::PAINTS.get() < 600);

            let reactions = cx.debug_bounds("reactions").unwrap();
            view.update(cx, |view, cx| {
                view.list.scroll_to(gpui::ListOffset {
                    item_ix: 0,
                    offset_in_item: card.size.height + reactions.size.height - px(600.),
                });
                cx.notify();
            });
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            let image = cx.debug_bounds("gallery-image").unwrap();
            assert!(image.top() >= px(0.));
            assert!(image.bottom() <= px(600.));
        }
    }
}
