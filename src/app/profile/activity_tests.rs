use gpui::{Context, IntoElement, Render, Styled, TestAppContext, Window, div, prelude::*, px};

use super::{ACTIVITY_PROMPTS, ProfilePalette, ProfileTheme, activity_empty, tab_panel};
use crate::theme::rpx;

struct ActivityLayout {
    width: f32,
    height: f32,
    prompt: usize,
    name: String,
    light: bool,
}

impl Render for ActivityLayout {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = self.light.then_some(ProfileTheme {
            primary: 0xF0E4F8,
            secondary: 0xDBC4EE,
        });
        div()
            .flex()
            .flex_col()
            .w(rpx(self.width))
            .h(rpx(self.height))
            .child(div().h(rpx(40.)).flex_none())
            .child(tab_panel(
                activity_empty(
                    &self.name,
                    &ACTIVITY_PROMPTS[self.prompt],
                    ProfilePalette::of(theme),
                )
                .into_any_element(),
            ))
    }
}

#[test]
fn activity_empty_state_has_no_card_surface() {
    for prompt in &ACTIVITY_PROMPTS {
        let mut empty = activity_empty("SampleUser", prompt, ProfilePalette::of(None));
        assert!(empty.style().background.is_none());
    }
}

#[gpui::test]
fn all_flavors_center_and_wrap_in_the_available_panel(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| ActivityLayout {
        width: 482.,
        height: 700.,
        prompt: 0,
        name: "-SampleUser".into(),
        light: false,
    });
    for (width, height, name) in [
        (482., 700., "-SampleUser"),
        (300., 420., "Sample Display Name For Wrapping"),
        (240., 260., "AReallyLongDisplayNameWithoutAnySpacesAtAll"),
    ] {
        for zoom in [1., 1.25, 1.5] {
            for (prompt, definition) in ACTIVITY_PROMPTS.iter().enumerate() {
                view.update(cx, |layout, cx| {
                    layout.width = width;
                    layout.height = height;
                    layout.name = name.into();
                    layout.prompt = prompt;
                    layout.light = zoom > 1.;
                    cx.notify();
                });
                cx.update(|window, cx| {
                    window.set_rem_size(px(16. * zoom));
                    window.refresh();
                    let _ = window.draw(cx);
                });
                let empty = cx.debug_bounds("profile-activity-empty").unwrap();
                let copy = cx.debug_bounds("profile-activity-copy").unwrap();
                let flavor = cx.debug_bounds("profile-activity-flavor").unwrap();
                assert!((copy.center().x - empty.center().x).abs() <= px(1.));
                assert!((copy.center().y - empty.center().y).abs() <= px(1.));
                assert!(copy.size.width <= px(360. * zoom + 1.));
                assert!(copy.left() >= empty.left() + px(16. * zoom - 1.));
                assert!(copy.right() <= empty.right() - px(16. * zoom - 1.));
                assert!(copy.top() >= empty.top());
                assert!(copy.bottom() <= empty.bottom());
                assert!(flavor.size.height >= px(40. * zoom - 1.));
                let heading = cx.debug_bounds("profile-activity-heading");
                assert_eq!(heading.is_some(), definition.heading);
                if let Some(heading) = heading {
                    assert!((flavor.top() - heading.bottom() - px(8. * zoom)).abs() <= px(1.));
                }
            }
        }
    }
}

#[gpui::test]
fn ordinary_tab_content_stays_top_aligned(cx: &mut TestAppContext) {
    struct Content;
    impl Render for Content {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().flex().flex_col().w(rpx(482.)).h(rpx(700.)).child(
                tab_panel(
                    div()
                        .h(rpx(80.))
                        .debug_selector(|| "tab-content".into())
                        .into_any_element(),
                )
                .debug_selector(|| "tab-panel".into()),
            )
        }
    }
    let (_, cx) = cx.add_window_view(|_, _| Content);
    cx.update(|window, cx| {
        window.set_rem_size(px(16.));
        let _ = window.draw(cx);
    });
    let panel = cx.debug_bounds("tab-panel").unwrap();
    let content = cx.debug_bounds("tab-content").unwrap();
    assert_eq!(content.top(), panel.top() + px(20.));
    assert_eq!(content.size.height, px(80.));
}

#[gpui::test]
fn mutual_icons_never_cover_the_count(cx: &mut TestAppContext) {
    struct Line;
    impl Render for Line {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let icons = (0..3).map(|_| div().size(rpx(16.)).rounded_full().into_any_element());
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(rpx(6.))
                .w(rpx(300.))
                .text_size(rpx(12.))
                .child(super::mutual_entry("mutual", icons, "16 Mutual Servers".into()))
        }
    }
    let (_, cx) = cx.add_window_view(|_, _| Line);
    cx.update(|window, cx| {
        window.set_rem_size(px(16.));
        let _ = window.draw(cx);
    });
    let icons = cx.debug_bounds("mutual-icons").unwrap();
    let text = cx.debug_bounds("mutual-text").unwrap();
    assert!(icons.size.width >= px(39.), "icons {:?}", icons);
    assert!(text.left() >= icons.right(), "icons {:?} text {:?}", icons, text);
}
