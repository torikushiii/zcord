use chrono::Local;
use gpui::{prelude::*, *};

use crate::{
    app::{
        DiscordApp,
        display_name::{self, Surface},
    },
    settings::RoleColors,
    store::Status,
    text::emoji,
    theme::{self, rpx},
};

const SAMPLE_ROLE_COLOR: u32 = 0xDD80F4;
const LINK: &str = "https://discord.com/accessibility";

pub(super) fn preview(app: &DiscordApp, cx: &App) -> AnyElement {
    let display = crate::settings::display();
    let compact = display.message_display_compact;
    let me = app.store.current_user_id;
    let name = app
        .store
        .member_name(app.selected_guild_id, me)
        .or_else(|| app.store.user(me).map(|user| user.name.as_str()))
        .unwrap_or("You")
        .to_owned();
    let role = app
        .store
        .member_color(app.selected_guild_id, me)
        .unwrap_or(SAMPLE_ROLE_COLOR);
    let time = Local::now()
        .format(crate::settings::clock_format("%-I:%M %p", "%H:%M"))
        .to_string();
    let sample = Sample {
        app,
        me,
        name: &name,
        role,
        time: &time,
        compact,
        compact_avatar: compact && display.display_compact_avatars,
    };
    let reactions = display
        .render_reactions
        .then(|| reactions().into_any_element());
    let gap = crate::settings::message_group_spacing(compact);
    let card = div()
        .flex()
        .justify_between()
        .gap(rpx(24.))
        .mt(rpx(8.))
        .pt(rpx(24.))
        .pb(rpx(16.))
        .pl(rpx(24.))
        .pr(rpx(32.))
        .rounded(rpx(8.))
        .border_1()
        .border_color(rgba(theme::border()))
        .bg(rgb(theme::base_lowest()))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(sample.message(
                    "preview-first",
                    StyledText::new("what happened to all the beans"),
                    reactions,
                    cx,
                ))
                .child(div().mt(rpx(gap as f32)).child(sample.message(
                    "preview-second",
                    linked_text(),
                    None,
                    cx,
                ))),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .items_end()
                .justify_between()
                .w(rpx(120.))
                .child(
                    div().flex().flex_col().gap(rpx(6.)).children(
                        [
                            (0, Status::Online),
                            (2, Status::Idle),
                            (4, Status::DoNotDisturb),
                        ]
                        .map(|(index, status)| {
                            theme::default_avatar(
                                index,
                                32.,
                                Some((status, theme::base_lowest())),
                                cx,
                            )
                        }),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .mt(rpx(20.))
                        .w_full()
                        .h(rpx(32.))
                        .rounded(rpx(8.))
                        .border_1()
                        .border_color(rgba(0xFFFFFF14))
                        .bg(rgb(theme::BLURPLE))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(0xFFFFFF))
                        .child("Example Button"),
                ),
        );
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::faint()))
                .child("Preview"),
        )
        .child(card)
        .into_any_element()
}

struct Sample<'a> {
    app: &'a DiscordApp,
    me: u64,
    name: &'a str,
    role: u32,
    time: &'a str,
    compact: bool,
    compact_avatar: bool,
}

impl Sample<'_> {
    fn message(
        &self,
        id: &'static str,
        body: StyledText,
        reactions: Option<AnyElement>,
        cx: &App,
    ) -> AnyElement {
        let font_size = crate::settings::chat_font_size();
        let text = div()
            .text_size(rpx(font_size))
            .line_height(rpx(font_size * 22. / 16.))
            .text_color(rgb(theme::text()));
        let body = text.child(body);
        if self.compact {
            return div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_baseline()
                        .gap(rpx(4.))
                        .child(self.timestamp())
                        .when(self.compact_avatar, |this| {
                            this.child(div().self_center().child(self.avatar(16., cx)))
                        })
                        .child(self.author(id, cx))
                        .child(body),
                )
                .children(reactions)
                .into_any_element();
        }
        div()
            .flex()
            .gap(rpx(16.))
            .child(div().mt(rpx(2.)).child(self.avatar(40., cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(rpx(4.))
                            .h(rpx(22.))
                            .child(self.author(id, cx))
                            .child(self.timestamp()),
                    )
                    .child(body)
                    .children(reactions),
            )
            .into_any_element()
    }

    fn avatar(&self, size: f32, cx: &App) -> impl IntoElement + use<> {
        theme::member_avatar(
            &self.app.store,
            self.app.selected_guild_id,
            self.me,
            size,
            None,
            false,
            cx,
        )
    }

    fn author(&self, id: &'static str, cx: &App) -> impl IntoElement + use<> {
        let mode = crate::settings::role_colors();
        let color = match mode {
            RoleColors::InNames => self.role,
            RoleColors::NextToNames | RoleColors::Hidden => theme::strong(),
        };
        let style = self
            .app
            .store
            .user(self.me)
            .and_then(|user| user.name_style.as_ref());
        div()
            .flex()
            .items_center()
            .gap(rpx(4.))
            .text_size(rpx(16.))
            .line_height(rpx(22.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(color))
            .child(display_name::render(
                id,
                self.name,
                style,
                Surface::FontOnly,
                cx,
            ))
            .when(mode == RoleColors::NextToNames, |this| {
                this.child(theme::role_dot(self.role))
            })
            .children(
                self.app
                    .store
                    .user(self.me)
                    .and_then(|user| user.tag.as_ref())
                    .map(|tag| theme::guild_tag(tag, cx)),
            )
    }

    fn timestamp(&self) -> impl IntoElement + use<> {
        div()
            .flex_none()
            .text_size(rpx(12.))
            .line_height(rpx(22.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::faint()))
            .child(self.time.to_owned())
    }
}

fn linked_text() -> StyledText {
    let lead = "here's a link ";
    let link = HighlightStyle {
        color: Some(rgb(theme::link()).into()),
        underline: crate::settings::underline_links().then(|| UnderlineStyle {
            thickness: px(1.),
            color: Some(rgb(theme::link()).into()),
            wavy: false,
        }),
        ..Default::default()
    };
    StyledText::new(format!("{lead}{LINK}"))
        .with_highlights(vec![(lead.len()..lead.len() + LINK.len(), link)])
}

fn reactions() -> impl IntoElement {
    let pill = |sequence: &str, count: u32, me: bool| {
        div()
            .flex()
            .items_center()
            .gap(rpx(6.))
            .h(rpx(30.))
            .px(rpx(8.))
            .rounded(rpx(8.))
            .border_1()
            .map(|this| {
                if me {
                    this.bg(rgba(theme::reaction_selected_bg()))
                        .border_color(rgb(theme::BLURPLE))
                } else {
                    this.bg(rgb(theme::elevated()))
                        .border_color(rgb(theme::elevated()))
                }
            })
            .children(
                emoji::image(sequence, theme::image_px(20.))
                    .map(|image| img(image).size(rpx(20.)).flex_none()),
            )
            .child(
                div()
                    .text_size(rpx(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(if me {
                        theme::mention_text()
                    } else {
                        theme::muted()
                    }))
                    .child(count.to_string()),
            )
    };
    div()
        .flex()
        .gap(rpx(4.))
        .mt(rpx(4.))
        .child(pill("🫘", 3, true))
        .child(pill("👀", 1, false))
}
