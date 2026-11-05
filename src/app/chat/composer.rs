use super::message::{author_name, message_author_name};
use super::*;
use crate::app::attachments::{MAX_FILES, Refusal, STAGED_PREVIEW, StagedFile, UploadButton};
use crate::app::mentions;
use gpui_component::{ActiveTheme, input::Paste};

const PREVIEW_MAX_BYTES: u64 = 20 * 1024 * 1024;
/// The typing indicator's strip, which overhangs the bottom of the message list.
pub(super) const TYPING_HEIGHT: f32 = 24.;

pub(crate) fn composer(app: &DiscordApp, cx: &App) -> impl IntoElement + use<> {
    let can_send = app.store.can_send(app.selected_channel_id);
    // Discord counts the sent form, so tokens count as their `<:name:id>` markup.
    let text = app.composer.read(cx).text();
    let length = if text.chars().any(composer_tokens::is_token_char) {
        composer_tokens::to_wire(&text.to_string()).chars().count()
    } else {
        text.len_chars()
    };
    let remaining = MESSAGE_LIMIT as i64 - length as i64;
    let picker_anchor = Rc::new(Cell::new(None));
    let staged = app.staged_files();
    let joined_top = app.reply.is_some() || !staged.is_empty();
    let paste = app.entity.clone();
    let composer_input = app.composer.clone();
    let send_button = crate::settings::get(cx).composer_send_button;
    div()
        .relative()
        .px_2()
        .pb_2()
        .flex_none()
        .on_prepaint({
            let anchor = picker_anchor.clone();
            move |bounds, window, _| {
                let gap = rpx(8.).to_pixels(window.rem_size());
                let right_inset = rpx(24.).to_pixels(window.rem_size());
                anchor.set(Some(point(
                    bounds.right() - right_inset,
                    bounds.top() - gap,
                )));
            }
        })
        .capture_action(move |_: &Paste, window, cx| {
            if !composer_input.read(cx).focus_handle(cx).is_focused(window) {
                return;
            }
            let handled = paste
                .update(cx, |app, cx| app.paste_clipboard_files(cx))
                .unwrap_or(false);
            if handled {
                cx.stop_propagation();
            }
        })
        .children(
            app.upload_refusal
                .map(|refusal| refusal_banner(app, refusal)),
        )
        .children(app.reply.as_ref().map(|reply| reply_banner(app, reply, cx)))
        .children((!staged.is_empty()).then(|| staged_row(app, staged, cx)))
        .child(
            div()
                .relative()
                .flex()
                .items_start()
                .gap_1()
                .min_h(rpx(56.))
                .pl(rpx(12.))
                .pr(rpx(8.))
                .map(|this| {
                    if joined_top {
                        this.rounded_b(rpx(8.))
                    } else {
                        this.rounded(rpx(8.))
                    }
                })
                .bg(rgb(theme::elevated()))
                .border_1()
                .border_color(rgba(theme::border()))
                .on_mouse_down(MouseButton::Right, {
                    let entity = app.entity.clone();
                    move |event, _, cx| {
                        cx.stop_propagation();
                        let position = event.position;
                        entity
                            .update(cx, |app, cx| {
                                app.open_composer_menu(ComposerTarget::Main, position, cx)
                            })
                            .ok();
                    }
                })
                .when(app.editing.is_none(), |this| {
                    this.children(mentions::suggestions(app, cx))
                        .children(emoji_picker::suggestions(app, cx))
                })
                .child(div().my(rpx(12.)).child({
                    let entity = app.entity.clone();
                    app.upload_tip(
                        UploadButton::Attach,
                        "Upload a File",
                        theme::icon_button("attach", "circle-plus", 24., theme::muted()),
                    )
                    .when(!can_send || staged.len() >= MAX_FILES, |this| {
                        this.opacity(0.5)
                    })
                    .when(can_send && staged.len() < MAX_FILES, |this| {
                        this.on_click(move |_, window, cx| {
                            entity.update(cx, |app, cx| app.pick_files(window, cx)).ok();
                        })
                    })
                }))
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .ml_4()
                        .child(
                            flush_textarea(Textarea::new(&app.composer))
                                .disabled(!can_send)
                                .py(rpx(17.))
                                .text_size(rpx(16.))
                                .line_height(rpx(22.))
                                .text_color(rgb(theme::text())),
                        )
                        .child(emoji_token_overlay(&app.composer)),
                )
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .my(rpx(12.))
                        .child({
                            let entity = app.entity.clone();
                            let picker_anchor = picker_anchor.clone();
                            theme::icon_button("gif", "image-play", 24., theme::muted())
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    let Some(position) = picker_anchor.get() else {
                                        return;
                                    };
                                    // The picker focuses its search; keep the root from taking it back.
                                    window.prevent_default();
                                    entity
                                        .update(cx, |app, cx| {
                                            app.toggle_gif_picker(position, window, cx)
                                        })
                                        .ok();
                                })
                        })
                        .child({
                            let entity = app.entity.clone();
                            let picker_anchor = picker_anchor.clone();
                            theme::icon_button("stickers", "sticker", 24., theme::muted())
                                .relative()
                                .child(theme::hover_tooltip("stickers-tip", "Stickers"))
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    let Some(position) = picker_anchor.get() else {
                                        return;
                                    };
                                    window.prevent_default();
                                    entity
                                        .update(cx, |app, cx| {
                                            app.toggle_sticker_picker(
                                                PickerTarget::Composer,
                                                position,
                                                window,
                                                cx,
                                            )
                                        })
                                        .ok();
                                })
                        })
                        .child({
                            let entity = app.entity.clone();
                            theme::icon_button("emoji", "smile", 24., theme::muted()).on_mouse_down(
                                MouseButton::Left,
                                move |_, window, cx| {
                                    let Some(position) = picker_anchor.get() else {
                                        return;
                                    };
                                    window.prevent_default();
                                    entity
                                        .update(cx, |app, cx| {
                                            app.toggle_emoji_picker(
                                                PickerTarget::Composer,
                                                position,
                                                window,
                                                cx,
                                            )
                                        })
                                        .ok();
                                },
                            )
                        })
                        .children(send_button.then(|| {
                            let entity = app.entity.clone();
                            send_icon("send", can_send).on_click(move |_, window, cx| {
                                entity
                                    .update(cx, |app, cx| app.send_message(window, cx))
                                    .ok();
                            })
                        })),
                )
                .when(remaining < 200, |this| {
                    this.child(
                        div()
                            .absolute()
                            .right(rpx(12.))
                            .bottom(rpx(4.))
                            .text_size(rpx(12.))
                            .text_color(rgb(if remaining < 0 {
                                theme::RED
                            } else {
                                theme::faint()
                            }))
                            .child(remaining.to_string()),
                    )
                }),
        )
        .children(typing_indicator(app))
}

/// The Send Message Button, shown when the composer's right-click menu turns it on.
pub(super) fn send_icon(key: &'static str, enabled: bool) -> Stateful<Div> {
    theme::icon_button(key, "send", 24., theme::muted())
        .relative()
        .when(!enabled, |this| this.opacity(0.5))
        .child(theme::hover_tooltip(
            SharedString::from(format!("{key}-tip")),
            "Send Message",
        ))
}

/// Paints custom emoji over their invisible composer tokens (see `composer_tokens`).
///
/// Token positions come from the textarea's layout, which it records while painting, so this
/// canvas must come after the textarea to read the current frame's layout instead of the last.
pub(super) fn emoji_token_overlay(input: &Entity<TextareaState>) -> impl IntoElement + use<> {
    let input = input.clone();
    canvas(
        |_, _, _| {},
        move |_, _, window, cx| {
            let (tokens, clip, selected) = {
                let state = input.read(cx);
                // Paint runs every frame; skip the text copy unless a token can be present.
                if !state.text().chars().any(composer_tokens::is_token_char) {
                    return;
                }
                // Keyed by occurrence, not offset, so typing before an emoji keeps its clock.
                let mut seen = std::collections::HashMap::<u64, usize>::new();
                let tokens: Vec<_> = composer_tokens::tokens_in(&state.value())
                    .into_iter()
                    .filter_map(|(range, token)| {
                        let occurrence = seen.entry(token.id).or_default();
                        let key = (token.id, *occurrence);
                        *occurrence += 1;
                        let bounds = state.range_to_bounds(&range)?;
                        Some((range, bounds, token, key))
                    })
                    .collect();
                (tokens, state.input_bounds(), state.selected_range())
            };
            if tokens.is_empty() {
                return;
            }
            let size = rpx(18.).to_pixels(window.rem_size());
            let pixels = theme::image_px(22.);
            window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                for (range, bounds, token, (id, occurrence)) in tokens {
                    if bounds.intersect(&window.content_mask().bounds).is_empty() {
                        continue;
                    }
                    // The token's variation selectors have no glyph in the UI font and draw as
                    // small boxes, so cover the whole token with the input background first.
                    window.paint_quad(fill(bounds, rgb(theme::elevated())));
                    if range.start >= selected.start && range.end <= selected.end {
                        window.paint_quad(fill(bounds, cx.theme().selection));
                    }
                    // Keep a still visible while loading or when animation is disabled.
                    let still = ImageSpec::Emoji {
                        id: token.id,
                        animated: false,
                    };
                    let image = token
                        .animated
                        .then_some(ImageSpec::Emoji {
                            id: token.id,
                            animated: true,
                        })
                        .and_then(|spec| animated::resolve(&spec, pixels, cx))
                        .or_else(|| match animated::state(&still, pixels, cx) {
                            animated::Load::Ready(image) => Some(image),
                            animated::Load::Pending | animated::Load::Failed => None,
                        });
                    let Some(image) = image else {
                        continue;
                    };
                    let frame = window.with_id(("composer-input", input.entity_id()), |window| {
                        window.with_global_id(
                            ElementId::Name(format!("composer-emoji-{id}-{occurrence}").into()),
                            |id, window| animated::frame_index(&image, id, window, cx),
                        )
                    });
                    // Centred on the whole token: the EM SPACE plus whatever width the
                    // selector boxes take.
                    let dimensions = image.size(frame);
                    let scale = size / px(dimensions.width.0.max(dimensions.height.0) as f32);
                    let fitted = gpui::size(
                        px(dimensions.width.0 as f32) * scale,
                        px(dimensions.height.0 as f32) * scale,
                    );
                    let origin = bounds.center() - point(fitted.width / 2., fitted.height / 2.);
                    let target = Bounds::new(origin, fitted);
                    window
                        .paint_image(target, Corners::default(), image, frame, false)
                        .ok();
                }
            });
        },
    )
    .absolute()
    .inset_0()
}

/// The padding gpui-component's text area keeps inside itself, which `appearance(false)` and
/// `p_0` leave in place.
pub(crate) const TEXTAREA_BUILTIN_INSET_X: f32 = 10.;
pub(crate) const TEXTAREA_BUILTIN_INSET_Y: f32 = 8.;

pub(super) fn flush_textarea(textarea: Textarea) -> Textarea {
    textarea
        .appearance(false)
        .bordered(false)
        .w_full()
        .px_0()
        .ml(px(-TEXTAREA_BUILTIN_INSET_X))
        .my(px(-TEXTAREA_BUILTIN_INSET_Y))
}

fn refusal_banner(app: &DiscordApp, refusal: Refusal) -> impl IntoElement + use<> {
    let (title, help) = match refusal {
        Refusal::Invalid => (
            "Invalid file type",
            "That file type is not supported.".to_owned(),
        ),
        Refusal::TooMany => (
            "Too many uploads!",
            format!("You can only upload {MAX_FILES} files at a time!"),
        ),
        Refusal::TooLarge(limit) => (
            "Your files are too powerful",
            format!(
                "Max file size is {} please.",
                super::media::file_size(limit)
            ),
        ),
    };
    let entity = app.entity.clone();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .mb_2()
        .px_3()
        .py_2()
        .rounded(rpx(8.))
        .bg(rgb(theme::RED))
        .text_color(rgb(theme::strong()))
        .text_size(rpx(14.))
        .child(
            div()
                .min_w_0()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(help),
        )
        .child(
            theme::icon_button("dismiss-refusal", "x", 16., theme::strong()).on_click(
                move |_, _, cx| {
                    entity
                        .update(cx, |app, cx| app.dismiss_upload_refusal(cx))
                        .ok();
                },
            ),
        )
}

pub(super) fn staged_row(
    app: &DiscordApp,
    files: &[StagedFile],
    cx: &App,
) -> impl IntoElement + use<> {
    div()
        .id("staged-files")
        .flex()
        .gap(rpx(24.))
        .pt(rpx(20.))
        .pl(rpx(10.))
        .pb(rpx(10.))
        // The action bar hangs 24 past the last tile, and a scroller clips it otherwise.
        .pr(rpx(34.))
        .overflow_x_scroll()
        .when(app.reply.is_none(), |this| this.rounded_t(rpx(8.)))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_b_0()
        .border_color(rgba(theme::border()))
        .children(files.iter().map(|file| staged_tile(app, file, cx)))
}

fn staged_preview(file: &StagedFile, cx: &App) -> Option<AnyElement> {
    let pixels = theme::image_px(STAGED_PREVIEW);
    let fitted = |image: animated::AnimatedImage| {
        image
            .size_full()
            .object_fit(ObjectFit::Contain)
            .id(("staged-preview", file.id))
            .into_any_element()
    };
    if file.content_type == "image/gif" {
        let spec = ImageSpec::LocalGif {
            path: file.path.clone(),
        };
        // A covered GIF would decode for nothing.
        let playing = (!file.spoiler)
            .then(|| animated::resolve(&spec, pixels, cx))
            .flatten();
        return match playing {
            Some(frames) => Some(fitted(animated::image(frames))),
            None => animated::poster(&spec, pixels, cx).map(fitted),
        };
    }
    if file.content_type.starts_with("video/") {
        let spec = ImageSpec::LocalVideo {
            path: file.path.clone(),
        };
        return crate::media::video::poster(&spec, pixels, cx).map(fitted);
    }
    let still = file.size <= PREVIEW_MAX_BYTES
        && matches!(
            file.content_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp"
        );
    still.then(|| {
        img(file.path.clone())
            .size_full()
            .object_fit(ObjectFit::Contain)
            .into_any_element()
    })
}

fn tile_shadow() -> BoxShadow {
    BoxShadow {
        color: rgba(theme::control_shadow()).into(),
        offset: point(px(0.), px(1.)),
        blur_radius: px(4.),
        spread_radius: px(0.),
        inset: false,
    }
}

fn staged_tile(app: &DiscordApp, file: &StagedFile, cx: &App) -> impl IntoElement + use<> {
    let id = file.id;
    let preview = staged_preview(file, cx);
    let spoiler_toggle = app.entity.clone();
    let open = app.entity.clone();
    let can_open = preview.is_some();
    let remove = app.entity.clone();
    let tile =
        div()
            .flex()
            .flex_col()
            .size_full()
            .p(rpx(8.))
            .rounded(rpx(8.))
            .bg(rgb(theme::surface_high()))
            .border_1()
            .border_color(rgba(theme::border()))
            .shadow(vec![tile_shadow()])
            .child(
                div()
                    .id(("staged-open", id))
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_none()
                    .h(rpx(174.))
                    .when(can_open, |this| {
                        this.cursor_pointer().on_click(move |_, _, cx| {
                            open.update(cx, |app, cx| app.open_staged_lightbox(id, cx))
                                .ok();
                        })
                    })
                    .rounded(rpx(8.))
                    .bg(rgba(theme::hover()))
                    .overflow_hidden()
                    .child(preview.unwrap_or_else(|| {
                        theme::icon("file", 48., theme::muted()).into_any_element()
                    }))
                    .when(file.spoiler, |this| {
                        this.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(rgb(theme::elevated()))
                                .text_size(rpx(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::strong()))
                                .child("SPOILER"),
                        )
                    }),
            )
            .child(
                div()
                    .mt(rpx(8.))
                    .truncate()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .text_color(rgb(theme::text()))
                    .child(file.name.clone()),
            );
    // gpui paints a border over its children, so the bar can't sit inside the tile.
    div()
        .relative()
        .flex_none()
        .size(rpx(217.78))
        .child(tile)
        .child(
            div()
                .absolute()
                .top(rpx(0.))
                .right(rpx(-24.))
                .flex()
                .rounded(rpx(4.))
                .bg(rgb(theme::toolbar_bg()))
                .border_1()
                .border_color(rgb(theme::toolbar_border()))
                .shadow(vec![tile_shadow()])
                .occlude()
                .child(
                    app.upload_tip(
                        UploadButton::Spoiler(id),
                        if file.spoiler {
                            "Remove Spoiler"
                        } else {
                            "Mark as Spoiler"
                        },
                        theme::icon_button_with_hover(
                            ("staged-spoiler", id),
                            if file.spoiler { "eye-off" } else { "eye" },
                            20.,
                            theme::muted(),
                            rgb(theme::toolbar_hover()).into(),
                        ),
                    )
                    .size(rpx(32.))
                    .on_click(move |_, _, cx| {
                        spoiler_toggle
                            .update(cx, |app, cx| app.toggle_staged_spoiler(id, cx))
                            .ok();
                    }),
                )
                .child(
                    app.upload_tip(
                        UploadButton::Remove(id),
                        "Remove Attachment",
                        theme::icon_button_with_hover(
                            ("staged-remove", id),
                            "trash-2",
                            20.,
                            theme::RED,
                            rgb(theme::toolbar_hover()).into(),
                        ),
                    )
                    .size(rpx(32.))
                    .on_click(move |_, _, cx| {
                        remove.update(cx, |app, cx| app.remove_staged(id, cx)).ok();
                    }),
                ),
        )
}

fn reply_banner(app: &DiscordApp, reply: &ReplyTarget, cx: &App) -> impl IntoElement + use<> {
    let entity = app.entity.clone();
    let own = reply.author_id == app.store.current_user_id;
    let target = app.channel().and_then(|channel| {
        channel
            .message_index(reply.message_id)
            .map(|index| &channel.messages[index])
    });
    let name = target.map_or_else(
        || {
            reply.webhook_name.as_ref().map_or_else(
                || author_name(app, reply.author_id),
                |name| name.to_string(),
            )
        },
        |message| message_author_name(app, message),
    );
    div()
        .flex()
        .items_center()
        .h(rpx(34.))
        .pl(rpx(16.))
        .pr(rpx(8.))
        .rounded_t(rpx(8.))
        .bg(rgba(theme::hover()))
        .border_1()
        .border_b_0()
        .border_color(rgba(theme::border()))
        .text_size(rpx(14.))
        .text_color(rgb(theme::muted()))
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .gap(rpx(4.))
                .child("Replying to")
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::text()))
                        .child(crate::app::display_name::render(
                            ("reply-target", reply.message_id),
                            &name,
                            app.store
                                .member_name_style(app.selected_guild_id, reply.author_id)
                                .filter(|_| {
                                    target.map_or_else(
                                        || reply.webhook_name.is_none(),
                                        |message| message.webhook.is_none(),
                                    )
                                }),
                            crate::app::display_name::Surface::FontOnly,
                            cx,
                        )),
                ),
        )
        .when(!own, |this| {
            let toggle = entity.clone();
            this.child(
                div()
                    .id("reply-mention")
                    .flex()
                    .items_center()
                    .gap(rpx(2.))
                    .px_2()
                    .cursor_pointer()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(if reply.mention {
                        theme::link()
                    } else {
                        theme::muted()
                    }))
                    .relative()
                    .child(theme::hover_tooltip(
                        "reply-mention-tip",
                        "Mention the author",
                    ))
                    .on_click(move |_, _, cx| {
                        toggle
                            .update(cx, |app, cx| app.toggle_reply_mention(cx))
                            .ok();
                    })
                    .child("@")
                    .child(if reply.mention { "ON" } else { "OFF" }),
            )
            .child(
                div()
                    .w(rpx(1.))
                    .h(rpx(16.))
                    .mx_1()
                    .bg(rgba(theme::border())),
            )
        })
        .child(
            theme::icon_button("reply-cancel", "x", 16., theme::muted()).on_click(
                move |_, _, cx| {
                    entity.update(cx, |app, cx| app.cancel_reply(cx)).ok();
                },
            ),
        )
}

fn typing_indicator(app: &DiscordApp) -> Option<impl IntoElement> {
    let names: Vec<String> = app
        .typing_users()
        .iter()
        .map(|(user_id, _)| author_name(app, *user_id))
        .collect();
    let bold = |name: &String| {
        div()
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(theme::text()))
            .child(name.clone())
    };
    let text = div().flex().min_w_0().overflow_hidden().whitespace_nowrap();
    let text = match names.as_slice() {
        [] => text,
        [one] => text.child(bold(one)).child("\u{a0}is typing…"),
        [one, two] => text
            .child(bold(one))
            .child("\u{a0}and\u{a0}")
            .child(bold(two))
            .child("\u{a0}are typing…"),
        [one, two, three] => text
            .child(bold(one))
            .child(",\u{a0}")
            .child(bold(two))
            .child(", and\u{a0}")
            .child(bold(three))
            .child("\u{a0}are typing…"),
        _ => text.child("Several people are typing…"),
    };
    if names.is_empty() {
        return None;
    }
    Some(
        div()
            .absolute()
            .bottom(relative(1.))
            .left(rpx(8.))
            .right(rpx(8.))
            .flex()
            .items_center()
            .gap(rpx(6.))
            .h(rpx(TYPING_HEIGHT))
            .pl(rpx(8.))
            .bg(rgb(theme::bg()))
            .text_size(rpx(12.))
            .text_color(rgb(theme::muted()))
            .child(
                div()
                    .flex()
                    .gap(rpx(2.))
                    .children(
                        (0..3).map(|_| div().size(rpx(4.)).rounded_full().bg(rgb(theme::text()))),
                    )
                    .with_animation(
                        "typing-dots",
                        Animation::new(Duration::from_millis(1200))
                            .repeat()
                            .with_easing(pulsating_between(0.3, 1.)),
                        |dots, delta| dots.opacity(delta),
                    ),
            )
            .child(text),
    )
}
