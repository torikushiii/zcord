use super::*;

pub(super) fn status_bubble(
    user_id: u64,
    status: &CustomStatus,
    prompt: bool,
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
    let bubble = |size: f32| {
        div()
            .absolute()
            .size(rpx(size))
            .rounded_full()
            .bg(rgb(palette.status_surface))
            .border_1()
            .border_color(palette.status_border)
            .shadow(shadow())
    };
    div()
        .relative()
        .flex_none()
        .ml(rpx(layout.status_left))
        .mr(rpx(layout.status_right))
        .mt(rpx(-layout.status_overlap))
        .flex()
        .child(
            div()
                .relative()
                .max_w(rpx(181.))
                .min_w(rpx(42.))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(rpx(16.))
                        .bg(rgb(palette.status_surface))
                        .border_1()
                        .border_color(palette.status_border)
                        .shadow(shadow()),
                )
                .child(bubble(20.).left(rpx(10.)).top(rpx(-8.)))
                .child(bubble(10.).left(rpx(-3.)).top(rpx(-15.)))
                .child(
                    div()
                        .relative()
                        .rounded(rpx(16.))
                        .overflow_hidden()
                        .border_1()
                        .border_color(hsla(0., 0., 0., 0.))
                        .child(status_body(user_id, status, prompt, palette, cx)),
                ),
        )
}

fn status_emoji_source(emoji: &StatusEmoji, size: f32, cx: &App) -> Option<ImageSource> {
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

fn status_body(
    user_id: u64,
    status: &CustomStatus,
    prompt: bool,
    palette: ProfilePalette,
    cx: &App,
) -> Div {
    let text = status.text.as_deref().unwrap_or_default();
    let body = div()
        .min_w_0()
        .px(rpx(12.))
        .py(rpx(8.))
        .rounded(rpx(15.))
        .bg(rgb(palette.status_surface))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::NORMAL)
        .text_color(palette.status_text);
    if prompt {
        return body
            .cursor_pointer()
            .flex()
            .items_start()
            .gap(rpx(4.))
            .child(theme::icon("circle-plus-filled", 16., palette.muted).mt(rpx(1.)))
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
