use gpui::{prelude::*, *};
use gpui_component::input::Input;

use crate::{
    app::DiscordApp,
    data::{Attachment, Embed, Message},
    images,
    theme::{self, rpx},
};

const GROUP_MINUTES: u32 = 7;
const GUTTER: f32 = 72.;
const BOTTOM_SPACER: f32 = 24.;

pub fn render(app: &DiscordApp, cx: &mut Context<DiscordApp>) -> impl IntoElement + use<> {
    let channel = app.channel();
    let channel_name = channel.map_or("", |channel| &channel.name).to_owned();
    let guild_name = app.guild().map_or("", |guild| &guild.name).to_owned();
    let entity = cx.entity();
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(rpx(48.))
                .flex_none()
                .pl_4()
                .pr_2()
                .border_b_1()
                .border_color(rgb(theme::BORDER))
                .child(theme::icon("hash", 20., theme::FAINT))
                .child(
                    div()
                        .flex_none()
                        .text_size(rpx(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::STRONG))
                        .child(channel_name.clone()),
                )
                .child(div().flex_1())
                .child(theme::icon_button(
                    "threads",
                    "messages-square",
                    20.,
                    theme::MUTED,
                ))
                .child(theme::icon_button(
                    "notifications",
                    "bell",
                    20.,
                    theme::MUTED,
                ))
                .child(theme::icon_button("pins", "pin", 20., theme::MUTED))
                .child(
                    theme::icon_button(
                        "members",
                        "users",
                        20.,
                        if app.show_members {
                            theme::STRONG
                        } else {
                            theme::MUTED
                        },
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_members(cx))),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .ml_2()
                        .w(rpx(244.))
                        .h(rpx(32.))
                        .px_2()
                        .rounded(rpx(8.))
                        .bg(rgb(theme::ELEVATED))
                        .border_1()
                        .border_color(rgb(theme::BORDER))
                        .text_size(rpx(14.))
                        .text_color(rgb(theme::FAINT))
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .child(format!("Search {guild_name}")),
                        )
                        .child(theme::icon("search", 16., theme::MUTED)),
                ),
        )
        .map(|this| {
            if channel.is_some_and(|channel| app.live && !channel.messages_loaded) {
                let label = if app.is_loading(app.selected_channel_id) {
                    "Loading messages…"
                } else {
                    "Couldn't load messages."
                };
                this.child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(theme::MUTED))
                        .child(label),
                )
            } else {
                this.child(
                    list(app.messages.clone(), move |index, _, cx| {
                        render_row(entity.read(cx), index, cx)
                    })
                    .flex_1()
                    .min_h_0()
                    .w_full(),
                )
            }
        })
        .child(
            div().px_2().pb_2().flex_none().child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .min_h(rpx(56.))
                    .pl(rpx(12.))
                    .pr(rpx(8.))
                    .rounded(rpx(8.))
                    .bg(rgb(theme::ELEVATED))
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .child(theme::icon_button(
                        "attach",
                        "circle-plus",
                        24.,
                        theme::MUTED,
                    ))
                    .child(
                        div().flex_1().min_w_0().ml_4().child(
                            Input::new(&app.composer)
                                .appearance(false)
                                .w_full()
                                .h(rpx(56.))
                                .px_0()
                                .py(rpx(17.))
                                .text_size(rpx(16.))
                                .line_height(rpx(22.))
                                .text_color(rgb(theme::TEXT)),
                        ),
                    )
                    .child(theme::icon_button("gift", "gift", 24., theme::MUTED))
                    .child(theme::icon_button(
                        "translate",
                        "languages",
                        24.,
                        theme::MUTED,
                    ))
                    .child(theme::icon_button("gif", "image-play", 24., theme::MUTED))
                    .child(theme::icon_button("stickers", "sticker", 24., theme::MUTED))
                    .child(theme::icon_button("emoji", "smile", 24., theme::MUTED))
                    .child(theme::icon_button("apps", "layout-grid", 24., theme::MUTED)),
            ),
        )
}

fn render_row(app: &DiscordApp, index: usize, cx: &App) -> AnyElement {
    let Some(messages) = app.channel().map(|channel| &channel.messages) else {
        return div().into_any_element();
    };
    let Some(message) = messages.get(index) else {
        return div().into_any_element();
    };
    let previous = index.checked_sub(1).and_then(|index| messages.get(index));
    let new_day = previous.is_some_and(|previous| !previous.timestamp.same_day(message.timestamp));
    let grouped = previous.is_some_and(|previous| {
        !new_day
            && previous.author_id == message.author_id
            && message.timestamp.minutes_since(previous.timestamp) < GROUP_MINUTES
    });
    div()
        .w_full()
        .when(new_day, |this| this.child(date_divider(message)))
        .when(index + 1 == messages.len(), |this| {
            this.pb(rpx(BOTTOM_SPACER))
        })
        .child(render_message(app, message, grouped, cx))
        .into_any_element()
}

fn date_divider(message: &Message) -> impl IntoElement {
    let line = || div().flex_1().h(rpx(1.)).bg(rgb(theme::BORDER));
    div()
        .flex()
        .items_center()
        .gap_2()
        .mx_4()
        .mt_6()
        .mb_2()
        .child(line())
        .child(
            div()
                .text_size(rpx(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::FAINT))
                .child(message.timestamp.date()),
        )
        .child(line())
}

fn render_message(
    app: &DiscordApp,
    message: &Message,
    grouped: bool,
    cx: &App,
) -> impl IntoElement {
    let user = app.store.user(message.author_id);
    let name_color: Hsla = user
        .and_then(|user| user.role_color)
        .map_or(rgb(theme::STRONG).into(), |color| rgb(color).into());
    div()
        .id(("message", message.id))
        .group("message")
        .relative()
        .flex()
        .w_full()
        .pr_4()
        .py(rpx(2.))
        .when(!grouped, |this| this.mt_4())
        .hover(|style| style.bg(rgb(theme::HOVER)))
        .child(
            div()
                .flex()
                .flex_none()
                .justify_center()
                .w(rpx(GUTTER))
                .map(|gutter| {
                    if grouped {
                        gutter.pt(rpx(4.)).child(
                            div()
                                .text_size(rpx(11.))
                                .text_color(transparent_black())
                                .group_hover("message", |style| style.text_color(rgb(theme::FAINT)))
                                .child(message.timestamp.time()),
                        )
                    } else {
                        gutter.pt(rpx(2.)).child(theme::user_avatar(
                            user,
                            message.author_id,
                            40.,
                            None,
                            cx,
                        ))
                    }
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .when(!grouped, |this| {
                    this.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rpx(6.))
                            .child(
                                div()
                                    .text_size(rpx(16.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(name_color)
                                    .cursor_pointer()
                                    .child(
                                        user.map_or("Unknown user", |user| &user.name).to_owned(),
                                    ),
                            )
                            .when(user.is_some_and(|user| user.bot), |this| {
                                this.child(theme::app_tag())
                            })
                            .children(
                                user.and_then(|user| user.tag.as_deref())
                                    .map(theme::guild_tag),
                            )
                            .child(
                                div()
                                    .text_size(rpx(12.))
                                    .text_color(rgb(theme::FAINT))
                                    .child(message.timestamp.full()),
                            ),
                    )
                })
                .when(!message.content.is_empty(), |this| {
                    this.child(
                        div()
                            .w_full()
                            .whitespace_normal()
                            .text_size(rpx(16.))
                            .line_height(rpx(22.))
                            .child(message.content.clone()),
                    )
                })
                .children(message.embed.as_ref().map(render_embed))
                .children(
                    message
                        .attachments
                        .iter()
                        .map(|attachment| render_attachment(attachment, cx)),
                ),
        )
}

const MEDIA_MAX: (f32, f32) = (550., 350.);

fn render_attachment(attachment: &Attachment, cx: &App) -> AnyElement {
    let Some(spec) = attachment.image_spec() else {
        return file_card(attachment).into_any_element();
    };
    let (width, height) = (
        attachment.width.unwrap_or(1).max(1) as f32,
        attachment.height.unwrap_or(1).max(1) as f32,
    );
    let scale = (MEDIA_MAX.0 / width).min(MEDIA_MAX.1 / height).min(1.);
    let (width, height) = ((width * scale).round(), (height * scale).round());
    let image = images::resolve(&spec, theme::image_px(width.max(height)), cx);
    div()
        .mt_1()
        .w(rpx(width))
        .h(rpx(height))
        .rounded(rpx(8.))
        .overflow_hidden()
        .bg(rgb(theme::ELEVATED))
        .children(image.map(|path| img(path).size_full().rounded(rpx(8.))))
        .into_any_element()
}

fn file_card(attachment: &Attachment) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_center()
        .gap_3()
        .mt_1()
        .max_w(rpx(432.))
        .p_3()
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .child(theme::icon("file", 32., theme::MUTED))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_color(rgb(0x00A8FC))
                        .child(attachment.filename.clone()),
                )
                .child(
                    div()
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::FAINT))
                        .child(file_size(attachment.size)),
                ),
        )
}

fn file_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} bytes"),
        1024..1_048_576 => format!("{:.2} KB", bytes as f64 / 1024.),
        _ => format!("{:.2} MB", bytes as f64 / 1_048_576.),
    }
}

fn render_embed(embed: &Embed) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .max_w(rpx(520.))
        .mt_1()
        .py_2()
        .pl_3()
        .pr_4()
        .rounded(rpx(4.))
        .border_l_4()
        .border_color(rgb(embed.color))
        .bg(rgb(theme::ELEVATED))
        .child(
            div()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::STRONG))
                .child(embed.title.clone()),
        )
        .children(embed.lines.iter().map(|line| {
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::TEXT))
                .child(line.clone())
        }))
        .child(
            div()
                .mt_1()
                .text_size(rpx(12.))
                .text_color(rgb(theme::MUTED))
                .child(embed.footer.clone()),
        )
}
