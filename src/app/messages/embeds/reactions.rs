use super::*;

pub(crate) fn emoji_element(emoji: &ReactionEmoji, size: f32, cx: &App) -> AnyElement {
    match emoji.id {
        Some(id) => {
            let spec = images::ImageSpec::Emoji {
                id,
                animated: emoji.animated,
            };
            let pixels = theme::image_px(size);
            let frames = emoji
                .animated
                .then(|| crate::media::animated::resolve(&spec, pixels, cx))
                .flatten();
            let still_spec = images::ImageSpec::Emoji {
                id,
                animated: false,
            };
            let child = match frames {
                Some(image) => Some(
                    crate::media::animated::image(image)
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .id(("emoji", id))
                        .into_any_element(),
                ),
                None => images::resolve(&still_spec, pixels, cx).map(|path| {
                    img(path)
                        .id(("emoji", id))
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .into_any_element()
                }),
            };
            div()
                .size(rpx(size))
                .flex_none()
                .children(child)
                .into_any_element()
        }
        None => match emoji::image(&emoji.name, theme::image_px(size)) {
            Some(image) => img(image)
                .size(rpx(size))
                .flex_none()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None => div()
                .flex_none()
                .text_size(rpx(size * 0.85))
                .line_height(rpx(size))
                .child(emoji.name.clone())
                .into_any_element(),
        },
    }
}

fn reactor_avatars(
    app: &DiscordApp,
    message_id: u64,
    reaction: &Reaction,
    cx: &App,
) -> impl IntoElement + use<> {
    const AVATAR: f32 = 21.6;
    const OVERLAP: f32 = 5.4;
    let users = app.known_reactors(message_id, reaction);
    let rest = (reaction.count as usize).saturating_sub(users.len());
    let chip = rest > 0 && !users.is_empty();
    div()
        .flex()
        .flex_none()
        .items_center()
        .ml(rpx(6.))
        .mr(rpx(4.))
        .children(users.iter().enumerate().map(|(index, user)| {
            let spec = crate::store::avatar_spec(
                user.id,
                user.avatar.as_deref(),
                user.discriminator.as_deref(),
            );
            let covered = chip || index + 1 < users.len();
            // Negative margins make the pill measure short, so an overlapped avatar
            // gets a narrower slot and spills into the next one instead.
            div()
                .flex_none()
                .h(rpx(AVATAR))
                .w(rpx(if covered { AVATAR - OVERLAP } else { AVATAR }))
                .child(theme::spec_avatar(
                    &spec,
                    user.id,
                    &user.username,
                    AVATAR,
                    cx,
                ))
        }))
        .when(chip, |this| {
            this.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(rpx(AVATAR))
                    .px(rpx(7.2))
                    .rounded_full()
                    .bg(rgb(theme::base_lowest()))
                    .text_size(rpx(10.8))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::text()))
                    .child(format!("+{rest}")),
            )
        })
}

pub fn render_reactions(
    app: &DiscordApp,
    message_id: u64,
    reactions: &[Reaction],
    cx: &App,
) -> Option<AnyElement> {
    if reactions.is_empty() || !crate::settings::display().render_reactions {
        return None;
    }
    let entity = app.entity.clone();
    let open = app.entity.clone();
    let tip_shown = app.hover.add_reaction == Some(message_id);
    let picking = app.picking_reaction_for(message_id);
    let who_reacted = crate::settings::who_reacted();
    let wanted = (who_reacted
        && reactions
            .iter()
            .any(|reaction| !app.reactors_settled(message_id, reaction)))
    .then(|| (app.entity.clone(), reactions.to_vec()));
    let pill = || {
        div()
            .flex()
            .items_center()
            .gap(rpx(6.))
            .h(rpx(30.))
            .px(rpx(6.))
            .rounded(rpx(8.))
            .border_1()
    };
    Some(
        div()
            .flex()
            .flex_wrap()
            .gap(rpx(4.))
            .mt(rpx(4.))
            .when_some(wanted, |this, (entity, reactions)| {
                this.on_prepaint(move |_, _, cx| {
                    let (entity, reactions) = (entity.clone(), reactions.clone());
                    cx.defer(move |cx| {
                        entity
                            .update(cx, |app, cx| app.want_reactors(message_id, &reactions, cx))
                            .ok();
                    });
                })
            })
            .children(reactions.iter().map(|reaction| {
                let (click, hover) = (entity.clone(), entity.clone());
                let (emoji, hovered) = (reaction.emoji.clone(), reaction.clone());
                pill()
                    .id(format!(
                        "reaction-{message_id}-{}",
                        crate::app::reaction_emoji_text(&reaction.emoji)
                    ))
                    .relative()
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        click
                            .update(cx, |app, cx| {
                                app.toggle_reaction(message_id, emoji.clone(), cx)
                            })
                            .ok();
                    })
                    .on_hover(move |over, _, cx| {
                        hover
                            .update(cx, |app, cx| {
                                app.set_reaction_hover(message_id, &hovered, *over, cx)
                            })
                            .ok();
                    })
                    .children(app.reaction_tooltip(message_id, reaction, cx))
                    .map(|this| {
                        if reaction.me {
                            this.bg(rgba(theme::reaction_selected_bg()))
                                .border_color(rgb(theme::BLURPLE))
                        } else {
                            this.bg(rgb(theme::elevated()))
                                .border_color(rgb(theme::elevated()))
                        }
                    })
                    .child(emoji_element(&reaction.emoji, 20., cx))
                    .child(
                        div()
                            .text_size(rpx(16.))
                            .line_height(rpx(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(if reaction.me {
                                theme::mention_text()
                            } else {
                                theme::muted()
                            }))
                            .child(reaction.count.to_string()),
                    )
                    .when(who_reacted, |this| {
                        this.child(reactor_avatars(app, message_id, reaction, cx))
                    })
            }))
            .child(
                pill()
                    .id(("add-reaction", message_id))
                    .bg(rgb(theme::elevated()))
                    .border_color(rgb(theme::elevated()))
                    .opacity(if picking { 1. } else { 0. })
                    .group_hover("message", |style| style.opacity(1.))
                    .hover(|style| style.bg(rgba(theme::selected())))
                    .cursor_pointer()
                    .relative()
                    .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        let position = event.position - point(px(0.), px(8.));
                        window.prevent_default();
                        open.update(cx, |app, cx| {
                            app.toggle_emoji_picker(
                                PickerTarget::Reaction(message_id),
                                position,
                                window,
                                cx,
                            )
                        })
                        .ok();
                    })
                    .on_hover(move |hovered, _, cx| {
                        entity
                            .update(cx, |app, cx| {
                                app.set_add_reaction_hover(message_id, *hovered, cx)
                            })
                            .ok();
                    })
                    .child(theme::icon("smile-plus", 20., theme::muted()))
                    .when(tip_shown && !picking, |this| {
                        this.child(theme::tooltip_above("Add Reaction"))
                    }),
            )
            .into_any_element(),
    )
}
