use super::*;

const GROUP: &str = "profile-status-bubble";
const TOOLBAR_RISE: f32 = 20.;

pub(super) fn status_bubble(
    user_id: u64,
    status: &CustomStatus,
    prompt: bool,
    edit: Option<WeakEntity<DiscordApp>>,
    layout: &Layout,
    palette: ProfilePalette,
    cx: &App,
) -> impl IntoElement + use<> {
    let shadow = || {
        vec![
            BoxShadow::new(
                px(0.),
                px(1.),
                hsla(0., 0., 0., if palette.surface_light { 0.08 } else { 0.14 }),
            )
            .blur_radius(px(4.)),
        ]
    };
    let editable = edit.is_some();
    let hovered = hover_tint(palette.status_surface, palette.surface_light);
    let surface = move |this: Div| {
        this.bg(rgb(palette.status_surface)).when(editable, |this| {
            this.group_hover(GROUP, move |style| style.bg(rgb(hovered)))
        })
    };
    let bubble = |size: f32| {
        surface(div())
            .absolute()
            .size(rpx(size))
            .rounded_full()
            .border_1()
            .border_color(palette.status_border)
            .shadow(shadow())
    };
    let toolbar = edit.clone().filter(|_| !prompt).map(toolbar);
    let open_editor = edit.filter(|_| prompt);
    let clickable = open_editor.is_some();
    div()
        .id(("profile-status-bubble", user_id))
        .group(GROUP)
        .relative()
        .flex_none()
        .ml(rpx(layout.status_left))
        .mr(rpx(layout.status_right))
        // The hover area reaches up over the toolbar, which sits above the bubble.
        .mt(rpx(-layout.status_overlap - TOOLBAR_RISE))
        .pt(rpx(TOOLBAR_RISE))
        .flex()
        .child(
            div()
                .relative()
                .max_w(rpx(181.))
                .min_w(rpx(42.))
                .child(
                    surface(div())
                        .absolute()
                        .inset_0()
                        .rounded(rpx(16.))
                        .border_1()
                        .border_color(palette.status_border)
                        .shadow(shadow()),
                )
                .child(bubble(20.).left(rpx(10.)).top(rpx(-8.)))
                .child(bubble(10.).left(rpx(-3.)).top(rpx(-15.)))
                .child(
                    div()
                        .id(("profile-status-body", user_id))
                        .relative()
                        .rounded(rpx(16.))
                        .overflow_hidden()
                        .border_1()
                        .border_color(hsla(0., 0., 0., 0.))
                        .when_some(open_editor, |this, entity| {
                            this.on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                entity
                                    .update(cx, |app, cx| app.open_status_editor(window, cx))
                                    .ok();
                            })
                        })
                        .child(surface(status_body(
                            user_id, status, prompt, clickable, palette, cx,
                        ))),
                )
                .children(toolbar),
        )
}

fn toolbar(entity: WeakEntity<DiscordApp>) -> Div {
    let edit = entity.clone();
    let tool = |id: &'static str, icon: &'static str, label: &'static str| {
        div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .flex()
            .items_center()
            .justify_center()
            .h(rpx(24.))
            .w(rpx(26.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(0xFFFFFF14)))
            .child(theme::icon(icon, 16., theme::text()))
    };
    div()
        .absolute()
        .top(rpx(-TOOLBAR_RISE))
        .right(rpx(8.))
        .flex()
        .p(rpx(2.))
        .rounded_full()
        .bg(hsla(0., 0., 0., 0.48))
        .border_1()
        .border_color(rgba(0x97979F0A))
        .shadow(vec![
            BoxShadow::new(px(0.), px(1.), hsla(0., 0., 0., 0.14)).blur_radius(px(4.)),
        ])
        .invisible()
        .group_hover(GROUP, |style| style.visible())
        .child(
            tool("profile-status-edit", "pencil", "Edit custom status")
                .pl(rpx(2.))
                .rounded_l_full()
                .rounded_r(rpx(6.))
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    edit.update(cx, |app, cx| app.open_status_editor(window, cx))
                        .ok();
                }),
        )
        .child(
            tool("profile-status-clear", "trash-2", "Clear custom status")
                .pr(rpx(2.))
                .rounded_l(rpx(6.))
                .rounded_r_full()
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    entity
                        .update(cx, |app, cx| app.clear_custom_status(cx))
                        .ok();
                }),
        )
}

pub(in crate::app::profile) fn status_emoji_source(
    emoji: &StatusEmoji,
    size: f32,
    cx: &App,
) -> Option<ImageSource> {
    match emoji {
        StatusEmoji::Unicode(sequence) => {
            crate::text::emoji::image(sequence, theme::image_px(size)).map(Into::into)
        }
        StatusEmoji::Custom { id, animated, .. } => images::resolve(
            &ImageSpec::Emoji {
                id: *id,
                animated: *animated && crate::settings::emoji_animation(),
            },
            theme::image_px(size),
            cx,
        )
        .map(Into::into),
    }
}

fn hover_tint(color: u32, light: bool) -> u32 {
    let target = if light { 0. } else { 255. };
    let mix = |shift: u32| {
        let channel = f32::from(((color >> shift) & 0xFF) as u8);
        ((channel + (target - channel) * 0.04).round() as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}

fn status_body(
    user_id: u64,
    status: &CustomStatus,
    prompt: bool,
    clickable: bool,
    palette: ProfilePalette,
    cx: &App,
) -> Div {
    let text = status.text.as_deref().unwrap_or_default();
    let body = div()
        .min_w_0()
        .px(rpx(12.))
        .py(rpx(8.))
        .rounded(rpx(15.))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::NORMAL)
        .text_color(palette.status_text);
    if prompt {
        return body
            .when(clickable, |this| this.cursor_pointer())
            .flex()
            .items_start()
            .gap(rpx(4.))
            .when(clickable, |this| {
                this.child(theme::icon("circle-plus-filled", 16., palette.muted).mt(rpx(1.)))
            })
            .child(
                div()
                    .min_w_0()
                    .line_clamp(2)
                    .italic()
                    .text_color(rgb(palette.muted))
                    .child(text.to_owned()),
            );
    }
    if text.trim().is_empty() {
        return body.child(
            div().size(rpx(32.)).children(
                status
                    .emoji
                    .as_ref()
                    .and_then(|emoji| status_emoji_source(emoji, 32., cx))
                    .map(|source| {
                        img(source)
                            .id(("profile-status-emoji", user_id))
                            .size_full()
                            .object_fit(ObjectFit::Contain)
                    }),
            ),
        );
    }
    let mut pieces = emoji_pieces(text, &PieceStyle::default());
    if let Some(emoji) = status.emoji.as_ref() {
        let fallback = match emoji {
            StatusEmoji::Unicode(sequence) => sequence.clone(),
            StatusEmoji::Custom { name, .. } => format!(":{name}:"),
        };
        pieces.insert(
            0,
            Piece {
                content: Content::Emoji(
                    status_emoji_source(emoji, 16., cx).map(EmojiSource::Still),
                    fallback.into(),
                ),
                style: PieceStyle::default(),
            },
        );
    }
    let content = rich_text(("profile-status", user_id), pieces)
        .emoji_size(rpx(16.))
        .emoji_gap(rpx(4.))
        .line_height(rpx(18.))
        .line_clamp(2)
        .selectable();
    body.cursor_text().child(
        div()
            .min_w_0()
            .max_h(rpx(36.))
            .overflow_hidden()
            .child(content),
    )
}
