use gpui::{prelude::*, *};
use gpui_component::ActiveTheme;

use super::{Call, DetailsTab, Encryption, PingSample, menu_frame};
use crate::{
    app::{DiscordApp, message_body::format_timestamp},
    theme::{self, rpx},
};

const WIDTH: f32 = 300.;
const GRAPH_HEIGHT: f32 = 80.;
const GRAPH_AXIS: f32 = 28.;
/// Discord's graph covers the last few minutes of the call.
const GRAPH_WINDOW: i64 = 5 * 60;

/// Discord's "Voice Details" popup: connection stats and the call's privacy code.
pub(super) fn card(
    app: &DiscordApp,
    call: &Call,
    tab: DetailsTab,
    closing: bool,
    cx: &App,
) -> Stateful<Div> {
    let code = call.encryption.privacy_code();
    let tab = if code.is_some() {
        tab
    } else {
        DetailsTab::Connection
    };
    // Both tabs lay out in the same grid cell, the hidden one invisible, so the popup keeps
    // the taller tab's height and switching tabs never resizes it.
    let privacy_shown = tab == DetailsTab::Privacy;
    let cell = |content: AnyElement, shown: bool| {
        div()
            .min_w_0()
            .w_full()
            .col_start(1)
            .row_start(1)
            .child(content)
            .when(!shown, |this| this.invisible())
    };
    let body = div()
        .min_w_0()
        .w_full()
        .grid()
        .grid_cols(1)
        .grid_rows(1)
        .child(cell(connection(call).into_any_element(), !privacy_shown))
        .children(code.map(|code| {
            cell(
                privacy(app, code, closing, cx).into_any_element(),
                privacy_shown,
            )
        }));
    menu_frame("voice-details")
        .w(rpx(WIDTH))
        .min_w_0()
        .whitespace_normal()
        .child(tabs(app, tab, code.is_some(), closing))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .w_full()
                .gap(rpx(12.))
                .p(rpx(16.))
                .child(body)
                .child(encryption_footer(&call.encryption, call.connected)),
        )
}

fn tabs(
    app: &DiscordApp,
    selected: DetailsTab,
    privacy_available: bool,
    closing: bool,
) -> impl IntoElement {
    let tab = |id: &'static str, label: &'static str, tab: DetailsTab| {
        let active = selected == tab;
        let entity = app.entity.clone();
        div()
            .id(id)
            .flex()
            .flex_col()
            .justify_between()
            .h(rpx(44.))
            .pt(rpx(12.))
            .cursor_pointer()
            .text_size(rpx(14.))
            .line_height(rpx(16.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(if active {
                theme::BLURPLE
            } else {
                theme::muted()
            }))
            .when(!active, |this| {
                this.hover(|style| style.text_color(rgb(theme::text())))
            })
            .when(!closing, |this| {
                this.on_click(move |_, _, cx| {
                    entity
                        .update(cx, |app, cx| app.select_details_tab(tab, cx))
                        .ok();
                })
            })
            .child(label)
            .child(
                div()
                    .h(rpx(2.))
                    .rounded_t(rpx(2.))
                    .when(active, |this| this.bg(rgb(theme::BLURPLE))),
            )
    };
    div()
        .flex()
        .gap(rpx(24.))
        .px(rpx(16.))
        .border_b_1()
        .border_color(rgba(theme::border()))
        .child(tab(
            "voice-details-connection",
            "Connection",
            DetailsTab::Connection,
        ))
        .when(privacy_available, |this| {
            this.child(tab("voice-details-privacy", "Privacy", DetailsTab::Privacy))
        })
}

fn connection(call: &Call) -> impl IntoElement {
    let now = chrono::Utc::now().timestamp();
    let recent: Vec<PingSample> = call
        .pings
        .iter()
        .copied()
        .filter(|sample| now - sample.unix <= GRAPH_WINDOW)
        .collect();
    let average = (!recent.is_empty()).then(|| {
        recent
            .iter()
            .map(|sample| u64::from(sample.ms))
            .sum::<u64>()
            / recent.len() as u64
    });
    // Discord names the server by the first label of its hostname.
    let server = call
        .server
        .as_deref()
        .and_then(|host| host.split('.').next())
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    let measuring = || "Measuring…".to_owned();
    div()
        .flex()
        .flex_col()
        .gap(rpx(12.))
        .child(graph(&recent, now))
        .children(server.map(|server| {
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(theme::strong()))
                .child(server)
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .child(stat(
                    "Average ping",
                    average.map_or_else(measuring, |ms| format!("{ms} ms")),
                ))
                .child(stat(
                    "Last ping",
                    call.last_ping()
                        .map_or_else(measuring, |ms| format!("{ms} ms")),
                ))
                .children(
                    call.packet_loss
                        .map(|percent| stat("Outbound packet loss rate", format!("{percent:.1}%"))),
                ),
        )
        .child(
            div()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::muted()))
                .child(linked_text(
                    "voice-troubleshooting",
                    "You may notice delayed audio at 250 ms or higher. You may sound robotic if \
                     your packet loss rate is over 10%. If the problem persists, disconnect and \
                     try again. See our ",
                    "troubleshooting guide",
                    " for more assistance.",
                    TROUBLESHOOTING_GUIDE,
                )),
        )
}

fn stat(label: &'static str, value: String) -> impl IntoElement {
    div()
        .flex()
        .gap(rpx(4.))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .text_color(rgb(theme::text()))
        .child(format!("{label}:"))
        .child(
            div()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(theme::strong()))
                .child(value),
        )
}

/// Stepped ping line over the last few minutes, with minute gridlines and a 0–max scale on
/// the right like Discord's.
fn graph(samples: &[PingSample], now: i64) -> impl IntoElement {
    let start = now - GRAPH_WINDOW;
    let peak = samples.iter().map(|sample| sample.ms).max().unwrap_or(0);
    // Two even steps from a short list of round numbers, so the labels stay readable.
    let step = [20, 25, 50, 75, 100, 150, 250, 500, 1000]
        .into_iter()
        .find(|step| step * 2 >= peak)
        .unwrap_or(peak.div_ceil(2));
    let top = step * 2;
    let minutes: Vec<i64> = (start / 60 + 1..=now / 60)
        .map(|minute| minute * 60)
        .collect();
    let x = move |unix: i64| ((unix - start) as f32 / GRAPH_WINDOW as f32).clamp(0., 1.);
    let points: Vec<(f32, f32)> = samples
        .iter()
        .map(|sample| (x(sample.unix), 1. - sample.ms.min(top) as f32 / top as f32))
        .collect();
    let gridlines: Vec<f32> = minutes.iter().map(|minute| x(*minute)).collect();
    let plot = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let at = |fx: f32, fy: f32| {
                bounds.origin + point(bounds.size.width * fx, bounds.size.height * fy)
            };
            for &fx in &gridlines {
                window.paint_quad(fill(
                    Bounds::new(at(fx, 0.), size(px(1.), bounds.size.height)),
                    rgba(theme::border()),
                ));
            }
            let Some(&(first_x, first_y)) = points.first() else {
                return;
            };
            let mut path = PathBuilder::stroke(px(1.5));
            path.move_to(at(first_x, first_y));
            let mut previous = first_y;
            for &(fx, fy) in &points[1..] {
                path.line_to(at(fx, previous));
                path.line_to(at(fx, fy));
                previous = fy;
            }
            path.line_to(at(1., previous));
            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(theme::BLURPLE));
            }
        },
    )
    .size_full();
    let y_label = |value: u32, top_edge: Option<f32>| {
        div()
            .absolute()
            .right_0()
            .w(rpx(GRAPH_AXIS - 4.))
            .map(|this| match top_edge {
                Some(fraction) => this.top(relative(fraction)).mt(rpx(-8.)),
                None => this.bottom(rpx(-8.)),
            })
            .text_size(rpx(12.))
            .line_height(rpx(16.))
            .text_color(rgb(theme::muted()))
            .child(value.to_string())
    };
    let x_labels = minutes.iter().map(move |&minute| {
        div()
            .absolute()
            .top_0()
            .left(relative(x(minute)))
            .ml(rpx(-36.))
            .w(rpx(72.))
            .flex()
            .justify_center()
            .text_size(rpx(12.))
            .line_height(rpx(16.))
            .text_color(rgb(theme::muted()))
            .child(format_timestamp(minute, 't'))
    });
    div()
        .flex()
        .flex_col()
        .gap(rpx(4.))
        .child(
            div()
                .relative()
                .h(rpx(GRAPH_HEIGHT))
                .pr(rpx(GRAPH_AXIS))
                .child(div().size_full().bg(rgb(theme::bg())).child(plot))
                .child(y_label(top, Some(0.)))
                .child(y_label(top / 2, Some(0.5)))
                .child(y_label(0, None)),
        )
        .child(
            div()
                .relative()
                .h(rpx(16.))
                .mr(rpx(GRAPH_AXIS))
                .children(x_labels),
        )
}

const E2EE_ARTICLE: &str = "https://discord.com/blog/meet-dave-e2ee-for-audio-video";
const TROUBLESHOOTING_GUIDE: &str = "https://support.discord.com/hc/en-us/articles/360045138471";

fn linked_text(
    id: &'static str,
    before: &str,
    link: &str,
    after: &str,
    url: &'static str,
) -> InteractiveText {
    let range = before.len()..before.len() + link.len();
    InteractiveText::new(
        id,
        StyledText::new(format!("{before}{link}{after}")).with_highlights([(
            range.clone(),
            HighlightStyle {
                color: Some(rgb(theme::link()).into()),
                ..Default::default()
            },
        )]),
    )
    .on_click(vec![range], move |_, _, cx| cx.open_url(url))
}

fn privacy(app: &DiscordApp, code: &str, closing: bool, cx: &App) -> impl IntoElement {
    let groups = code_groups(code);
    let intro = linked_text(
        "voice-privacy-learn-more",
        "A new code is generated when people join or leave this call. ",
        "Learn more about end-to-end encryption.",
        "",
        E2EE_ARTICLE,
    );
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .w_full()
        .gap(rpx(12.))
        .child(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .min_w_0()
                .w_full()
                .gap(rpx(4.))
                .child(
                    div()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(theme::strong()))
                        .child("Voice Privacy Code"),
                )
                .child(
                    div()
                        .min_w_0()
                        .w_full()
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .text_color(rgb(theme::muted()))
                        .child(intro),
                ),
        )
        .child(code_grid(&groups, cx))
        .child(copy_button(app, groups.join(" "), closing))
        .child(
            div()
                .flex_none()
                .min_w_0()
                .w_full()
                .pt(rpx(12.))
                .border_t_1()
                .border_color(rgba(theme::border()))
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::muted()))
                .child(
                    "Only you and your friends on this call can hear you. Nobody else \u{2013} \
                     not even Wumpus \u{2013} can listen in!",
                ),
        )
}

fn code_groups(code: &str) -> Vec<String> {
    let digits: Vec<char> = code.chars().filter(|c| !c.is_whitespace()).collect();
    digits
        .chunks(5)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

/// Discord lays the 30-digit code out as two rows of three five-digit groups.
fn code_grid(groups: &[String], cx: &App) -> Div {
    let mono = cx.theme().mono_font_family.clone();
    let row = |groups: &[String]| {
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .w_full()
            .items_center()
            .children(groups.iter().map(|group| {
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .justify_center()
                    .font_family(mono.clone())
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .text_color(rgb(theme::text()))
                    .child(group.clone())
            }))
    };
    let rows: Vec<_> = groups.chunks(3).map(row).collect();
    let mut grid = div()
        .flex()
        .flex_col()
        .min_w_0()
        .w_full()
        .h(rpx(156.))
        .px(rpx(16.))
        .py(rpx(24.))
        .rounded(rpx(8.))
        .bg(rgb(theme::bg()));
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            grid = grid.child(div().h(px(1.)).bg(rgba(theme::border())));
        }
        grid = grid.child(row);
    }
    grid
}

fn copy_button(app: &DiscordApp, code: String, closing: bool) -> impl IntoElement {
    let entity = app.entity.clone();
    let label = if app.voice.privacy_copied {
        "Copied!"
    } else {
        "Copy"
    };
    div()
        .id("voice-privacy-copy")
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .gap(rpx(8.))
        .h(rpx(32.))
        .rounded(rpx(8.))
        .border_1()
        .border_color(rgba(theme::border()))
        .bg(rgba(theme::hover()))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::selected())))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .when(!closing, |this| {
            this.on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                entity
                    .update(cx, |app, cx| app.mark_privacy_code_copied(cx))
                    .ok();
            })
        })
        .child(theme::icon("copy", 16., theme::strong()))
        .child(label)
}

fn encryption_footer(encryption: &Encryption, connected: bool) -> impl IntoElement {
    let (text, color) = match encryption {
        Encryption::On(_) => ("End-to-end encrypted", theme::GREEN),
        Encryption::Off => ("Not end-to-end encrypted", theme::muted()),
        Encryption::Pending if connected => ("Setting up end-to-end encryption…", theme::muted()),
        Encryption::Pending => ("Connecting to voice…", theme::muted()),
    };
    div()
        .flex()
        .items_center()
        .min_w_0()
        .w_full()
        .gap(rpx(6.))
        .px(rpx(8.))
        .py(rpx(6.))
        .rounded(rpx(8.))
        .bg(rgba(color << 8 | 0x14))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .child(theme::icon("lock", 12., color).flex_none())
        .child(div().flex_1().min_w_0().child(text))
}

#[cfg(test)]
mod tests {
    use super::{Encryption, code_groups};

    #[test]
    fn privacy_is_unavailable_until_a_nonempty_code_arrives() {
        for encryption in [
            Encryption::Pending,
            Encryption::Off,
            Encryption::On(String::new()),
            Encryption::On(" \t\n".into()),
        ] {
            assert_eq!(encryption.privacy_code(), None);
        }
        let encryption = Encryption::On("012345678901234567890123456789".into());
        assert_eq!(
            encryption.privacy_code(),
            Some("012345678901234567890123456789")
        );
    }

    #[test]
    fn display_and_clipboard_share_five_digit_groups_with_leading_zeroes() {
        let groups = code_groups("012345678901234567890123456789");
        assert_eq!(
            groups,
            ["01234", "56789", "01234", "56789", "01234", "56789"]
        );
        assert_eq!(groups.join(" "), "01234 56789 01234 56789 01234 56789");
    }

    #[test]
    fn existing_whitespace_does_not_change_display_or_copied_groups() {
        let groups = code_groups("01234\t56789 01234\n56789\u{2003}01234 56789");
        assert_eq!(groups.join(" "), "01234 56789 01234 56789 01234 56789");
    }
}
