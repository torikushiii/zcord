use super::*;

const IMAGE_OPTION: f32 = 211.;
const RECENT_AVATAR: f32 = 56.;
const RECENT_AVATAR_SLOTS: usize = 6;

pub(super) fn image_dialog(
    app: &DiscordApp,
    editor: &StylesEditor,
    target: ImageTarget,
    cx: &App,
) -> AnyElement {
    let user_id = app.store.current_user_id;
    let upload_premium = target == ImageTarget::Banner;
    let option = |id: &'static str, enabled: bool| {
        div()
            .id(id)
            .group(id)
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(rpx(IMAGE_OPTION))
            .rounded(rpx(8.))
            .bg(rgba(0x97979F33))
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(rgba(0x97979F4D)))
            })
            .when(!enabled, |this| this.opacity(0.5))
    };
    let label = |icon: AnyElement, text: &'static str, premium: bool| {
        div()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .gap(rpx(8.))
            .child(icon)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(4.))
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::strong()))
                    .when(premium, |this| {
                        this.child(theme::icon("nitro", 16., theme::strong()))
                    })
                    .child(text),
            )
    };
    let upload = option("image-upload", editor.nitro || !upload_premium)
        .when(editor.nitro || !upload_premium, |this| {
            this.on_click(on_app(app, move |app, cx| app.pick_style_image(target, cx)))
        })
        .child(label(
            theme::icon("image-plus", 24., theme::strong()).into_any_element(),
            "Upload Image",
            upload_premium,
        ));
    let half = IMAGE_OPTION / 2.;
    let mosaic = gif_picker::category_previews(app, 4, cx)
        .into_iter()
        .enumerate()
        .map(|(index, src)| {
            let corners = match index {
                0 => [8., 0., 0., 0.],
                1 => [0., 8., 0., 0.],
                2 => [0., 0., 0., 8.],
                _ => [0., 0., 8., 0.],
            };
            div()
                .absolute()
                .left(rpx(if index % 2 == 0 { 0. } else { half }))
                .top(rpx(if index < 2 { 0. } else { half }))
                .size(rpx(half))
                .child(gif_picker::gif_cell(&src, half, corners, cx))
        });
    let gif_badge = div()
        .px(rpx(4.))
        .h(rpx(18.))
        .flex()
        .items_center()
        .rounded(rpx(4.))
        .bg(rgb(0xFFFFFF))
        .text_size(rpx(11.))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(0x000000))
        .child("GIF");
    let entity = app.entity.clone();
    let gif = option("image-gif", editor.nitro)
        .when(editor.nitro, |this| {
            this.on_click(move |_, window, cx| {
                cx.stop_propagation();
                entity
                    .update(cx, |app, cx| {
                        app.close_style_dialog(cx);
                        app.open_style_gif_picker(target, window, cx);
                    })
                    .ok();
            })
        })
        .children(mosaic)
        .child(
            div()
                .absolute()
                .inset_0()
                .rounded(rpx(8.))
                .bg(rgba(0x00000080))
                .group_hover("image-gif", |style| style.bg(rgba(0x00000059))),
        )
        .child(label(gif_badge.into_any_element(), "Choose GIF", true));
    let recent = (target == ImageTarget::Avatar)
        .then(|| editor.recent_avatars.clone())
        .flatten()
        .map(|avatars| {
            let slots = avatars.iter().take(RECENT_AVATAR_SLOTS).map(|avatar| {
                let (id, storage_hash) = (avatar.id, avatar.storage_hash.clone());
                let spec = ImageSpec::ArchivedAvatar {
                    user_id,
                    avatar_id: id,
                    hash: storage_hash.to_string(),
                };
                let pixels = theme::image_px(RECENT_AVATAR);
                let image = spec
                    .animated()
                    .and_then(|animated| animated::resolve(&animated, pixels, cx))
                    .map(|frames| {
                        animated::image(frames)
                            .object_fit(ObjectFit::Cover)
                            .id(("recent-avatar-frames", id))
                            .size_full()
                            .rounded_full()
                            .into_any_element()
                    })
                    .or_else(|| {
                        images::resolve(&spec, pixels, cx).map(|path| {
                            img(path)
                                .size_full()
                                .rounded_full()
                                .object_fit(ObjectFit::Cover)
                                .into_any_element()
                        })
                    });
                div()
                    .id(("recent-avatar", id))
                    .size(rpx(RECENT_AVATAR))
                    .flex_none()
                    .rounded_full()
                    .bg(rgb(theme::base_lowest()))
                    .cursor_pointer()
                    .hover(|style| style.opacity(0.8))
                    .on_click(on_app(app, move |app, cx| {
                        app.edit_recent_avatar(id, storage_hash.clone(), cx)
                    }))
                    .children(image)
            });
            let empty = RECENT_AVATAR_SLOTS.saturating_sub(avatars.len());
            let placeholders = (0..empty).map(|_| {
                div()
                    .size(rpx(RECENT_AVATAR))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(rgba(0x97979F33))
                    .child(theme::icon("user", 28., theme::muted()))
            });
            div()
                .flex()
                .flex_col()
                .gap(rpx(12.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rpx(4.))
                        .child(section_title("Recent Avatars"))
                        .child(
                            div()
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .text_color(rgb(theme::muted()))
                                .child("Access your 6 most recent avatar uploads."),
                        ),
                )
                .child(
                    div()
                        .mt(rpx(4.))
                        .flex()
                        .gap(rpx(8.))
                        .children(slots)
                        .children(placeholders),
                )
        });
    let body = div()
        .flex()
        .flex_col()
        .gap(rpx(24.))
        .pt(rpx(16.))
        .px(rpx(24.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(8.))
                .child(div().flex().gap(rpx(8.)).child(upload).child(gif))
                .when(target == ImageTarget::Banner, |this| {
                    this.child(
                        div()
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .text_color(rgb(theme::muted()))
                            .child(
                                "Upload a PNG, JPG, or GIF under 10MB. Images should be at \
                                 least 680x240.",
                            ),
                    )
                }),
        )
        .children(recent)
        .into_any_element();
    let saved = app.saved_style_values();
    let has_own = match target {
        ImageTarget::Avatar => saved.avatar.is_some() || editor.draft.avatar.is_some(),
        ImageTarget::Banner => saved.banner.is_some() || editor.draft.banner.is_some(),
    };
    let footer = div().when(
        has_own && matches!(editor.scope, StyleScope::Guild(_)),
        |this| {
            this.child(
                button("image-remove", "Reset to main profile", false, 180.).on_click(on_app(
                    app,
                    move |app, cx| {
                        app.close_style_dialog(cx);
                        app.update_style_draft(
                            |draft| match target {
                                ImageTarget::Avatar => draft.avatar = Some(ImageChange::Remove),
                                ImageTarget::Banner => draft.banner = Some(ImageChange::Remove),
                            },
                            cx,
                        );
                    },
                )),
            )
        },
    );
    shell(app, editor, Some("Select an Image"), 480., body, footer)
}

/// A box-shadow would paint under the frame as well, so the dimming is a border this thick
/// whose inner edge is the frame.
const IMAGE_EDIT_DIM: f32 = 480.;
const ZOOM_SLIDER: f32 = 128.;
const ZOOM_TRACK_INSET: f32 = 5.;

pub(super) fn image_edit_dialog(
    app: &DiscordApp,
    editor: &StylesEditor,
    edit: &ImageEdit,
) -> AnyElement {
    let [image_width, image_height] = edit.image_size();
    let [frame_width, frame_height] = edit.frame_size();
    let frame_left = (STAGE[0] - frame_width) / 2.;
    let frame_top = (STAGE[1] - frame_height) / 2.;
    let round = edit.target == ImageTarget::Avatar;
    let (press, pan, release, release_out) = (
        app.entity.clone(),
        app.entity.clone(),
        app.entity.clone(),
        app.entity.clone(),
    );
    let stage = div()
        .id("image-edit-stage")
        .relative()
        .flex_none()
        .w(rpx(STAGE[0]))
        .h(rpx(STAGE[1]))
        .rounded(rpx(8.))
        .overflow_hidden()
        .bg(gpui::black())
        .cursor(CursorStyle::OpenHand)
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            let position = event.position;
            press
                .update(cx, |app, cx| app.start_image_drag(position, cx))
                .ok();
        })
        .on_mouse_move(move |event, window, cx| {
            if event.dragging() {
                let (position, scale) = (
                    event.position,
                    f32::from(rpx(1.).to_pixels(window.rem_size())),
                );
                pan.update(cx, |app, cx| app.drag_image(position, scale, cx))
                    .ok();
            }
        })
        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
            release.update(cx, |app, cx| app.end_image_drag(cx)).ok();
        })
        .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
            release_out
                .update(cx, |app, cx| app.end_image_drag(cx))
                .ok();
        })
        .child(
            animated::image(edit.preview.clone())
                .object_fit(ObjectFit::Fill)
                .id("image-edit-preview")
                .absolute()
                .left(rpx((STAGE[0] - image_width) / 2. + edit.offset[0]))
                .top(rpx((STAGE[1] - image_height) / 2. + edit.offset[1]))
                .w(rpx(image_width))
                .h(rpx(image_height)),
        )
        .child(
            div()
                .absolute()
                .left(rpx(frame_left - IMAGE_EDIT_DIM))
                .top(rpx(frame_top - IMAGE_EDIT_DIM))
                .w(rpx(frame_width + IMAGE_EDIT_DIM * 2.))
                .h(rpx(frame_height + IMAGE_EDIT_DIM * 2.))
                .border(rpx(IMAGE_EDIT_DIM))
                .border_color(rgba(0x2F313699))
                .when(round, |this| {
                    this.rounded(rpx(frame_width / 2. + IMAGE_EDIT_DIM))
                }),
        )
        .child(
            div()
                .absolute()
                .left(rpx(frame_left))
                .top(rpx(frame_top))
                .w(rpx(frame_width))
                .h(rpx(frame_height))
                .border(rpx(5.))
                .border_color(gpui::white())
                .when(round, |this| this.rounded_full()),
        );
    let slider_bounds = Rc::new(Cell::new(None::<Bounds<Pixels>>));
    let entity = app.entity.clone();
    let measured = slider_bounds.clone();
    let zoom_at = move |position: Point<Pixels>, cx: &mut App| {
        let Some(bar) = measured.get() else {
            return;
        };
        let inset = bar.size.width * (ZOOM_TRACK_INSET / ZOOM_SLIDER);
        let travel = bar.size.width - inset * 2.;
        let t = ((position.x - bar.origin.x - inset) / travel).clamp(0., 1.);
        entity
            .update(cx, |app, cx| {
                app.set_image_zoom(1. + t * (MAX_ZOOM - 1.), cx)
            })
            .ok();
    };
    let progress = (edit.zoom - 1.) / (MAX_ZOOM - 1.);
    let zoom_slider = {
        let (down, moved) = (zoom_at.clone(), zoom_at);
        div()
            .id("image-edit-zoom")
            .relative()
            .flex_none()
            .w(rpx(ZOOM_SLIDER))
            .h(rpx(24.))
            .cursor(CursorStyle::ResizeLeftRight)
            .child(measure_into(slider_bounds))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(rpx(10.))
                    .w_full()
                    .h(rpx(4.))
                    .rounded(rpx(4.))
                    .overflow_hidden()
                    .bg(rgb(0x383940))
                    .child(
                        div()
                            .h_full()
                            .w(rpx(ZOOM_SLIDER * progress))
                            .border_1()
                            .border_color(rgba(0xFFFFFF1A))
                            .bg(rgb(theme::BLURPLE)),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top(rpx(4.))
                    .left(rpx(ZOOM_TRACK_INSET
                        + (ZOOM_SLIDER - ZOOM_TRACK_INSET * 2.) * progress
                        - 8.))
                    .size(rpx(16.))
                    .rounded_full()
                    .border_1()
                    .border_color(rgba(0x97979F3D))
                    .bg(gpui::white())
                    .shadow(vec![
                        BoxShadow::new(px(0.), px(1.), hsla(0., 0., 0., 0.14)).blur_radius(px(4.)),
                    ]),
            )
            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                down(event.position, cx)
            })
            .on_mouse_move(move |event, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    moved(event.position, cx);
                }
            })
    };
    let zoom_icon = |size: f32| {
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(rpx(24.))
            .child(theme::icon("image-scale", size, 0x96979E))
    };
    let rotate = div()
        .id("image-edit-rotate")
        .group("image-edit-rotate")
        .relative()
        .flex_none()
        .p(rpx(6.))
        .cursor_pointer()
        .on_click(on_app(app, |app, cx| app.rotate_image_edit(cx)))
        .child(
            svg()
                .path("app/rotate-square.svg")
                .size(rpx(24.))
                .text_color(rgb(0x96979E))
                .group_hover("image-edit-rotate", |style| style.text_color(rgb(0xDCDCDF))),
        )
        .child(theme::hover_tooltip("image-edit-rotate-tooltip", "Rotate"));
    let tools = div()
        .flex()
        .items_center()
        .mx(rpx(-6.))
        .py(rpx(16.))
        .pl(rpx(36.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .gap(rpx(8.))
                .mx_auto()
                .child(zoom_icon(12.))
                .child(zoom_slider)
                .child(zoom_icon(24.)),
        )
        .child(rotate);
    let body = div()
        .flex()
        .flex_col()
        .pt(rpx(16.))
        .px(rpx(24.))
        .child(stage)
        .child(tools)
        .children(edit.error.clone().map(|error| {
            div()
                .pb(rpx(8.))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::RED))
                .child(error)
        }))
        .into_any_element();
    let edited = edit.edited() && !edit.applying;
    let footer = div()
        .child(
            button("image-edit-apply", "Apply", true, 100.)
                .when(edit.applying, |this| this.opacity(0.5).cursor_default())
                .when(!edit.applying, |this| {
                    this.on_click(on_app(app, |app, cx| app.apply_image_edit(cx)))
                }),
        )
        .child(
            button("image-edit-cancel", "Cancel", false, 100.)
                .on_click(on_app(app, |app, cx| app.close_style_dialog(cx))),
        )
        // The shell's footer runs right to left; this pushes Reset to the far left. An auto
        // margin would be simpler, but Taffy misplaces it in a reversed row.
        .child(div().flex_1())
        .child(
            div()
                .id("image-edit-reset")
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(0x6374F4))
                .when(!edited, |this| this.opacity(0.5))
                .when(edited, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.underline())
                        .on_click(on_app(app, |app, cx| app.reset_image_edit(cx)))
                })
                .child("Reset"),
        )
        .items_center();
    shell(app, editor, Some("Edit Image"), 480., body, footer)
}
