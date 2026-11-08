use super::editing_panel::{TILE_GROUP, removable, tile};
use super::*;

const HOVER_FADE: Duration = Duration::from_millis(220);

/// How far the nameplate tile has faded from its stand-in row (0) to your member row (1).
#[derive(Default)]
pub(crate) struct HoverFade {
    hovered: bool,
    /// Bumped on every change to restart the animation.
    changes: u64,
    /// The level when the current fade began, so leaving mid-fade reverses from there.
    from: f32,
    started: Option<Instant>,
}

impl HoverFade {
    fn target(&self) -> f32 {
        if self.hovered { 1. } else { 0. }
    }

    /// A partial fade takes the matching share of the full duration.
    fn duration(&self) -> Duration {
        HOVER_FADE.mul_f32((self.target() - self.from).abs())
    }

    fn level(&self, now: Instant) -> f32 {
        let duration = self.duration();
        let Some(started) = self.started.filter(|_| !duration.is_zero()) else {
            return self.target();
        };
        let progress = (now - started).as_secs_f32() / duration.as_secs_f32();
        self.from + (self.target() - self.from) * ease_in_out(progress.min(1.))
    }

    pub(crate) fn set(&mut self, hovered: bool) {
        if self.hovered == hovered {
            return;
        }
        let now = Instant::now();
        self.from = self.level(now);
        self.hovered = hovered;
        self.changes += 1;
        self.started = Some(now);
    }

    /// `layer` faded to the hover level, easing there after each change.
    pub(crate) fn fade(&self, id: &'static str, layer: Div) -> AnyElement {
        let (from, target, duration) = (self.from, self.target(), self.duration());
        if duration.is_zero() {
            return layer.opacity(target).into_any_element();
        }
        layer
            .with_animation(
                (id, self.changes),
                Animation::new(duration).with_easing(ease_in_out),
                move |layer, delta| layer.opacity(from + (target - from) * delta),
            )
            .into_any_element()
    }
}

/// A stand-in member row that cross-fades to your real one, with a remove button, on hover.
pub(super) fn nameplate_tile(
    app: &DiscordApp,
    editor: &StylesEditor,
    shown: &Shown,
    decoration: Option<&Decoration>,
    cx: &App,
) -> AnyElement {
    let user_id = app.store.current_user_id;
    let user = app.store.user(user_id);
    let hover = &editor.ui.nameplate_hover;
    let hovered = hover.hovered;
    let has_nameplate = removable(&editor.draft.nameplate, &app.saved_style_values().nameplate);
    let plate = shown
        .nameplate
        .and_then(|sku| nameplate_of(app, editor, sku))
        .and_then(|plate| {
            rounded_nameplate_layer(&plate, user_id, BAR_TILE_HEIGHT, hovered, TILE_RADIUS, cx)
        });
    let (from, target, changes, duration) =
        (hover.from, hover.target(), hover.changes, hover.duration());
    let opacity = move |level: f32, shown_when_hovered: bool| {
        if shown_when_hovered {
            level
        } else {
            1. - level
        }
    };
    let fade = move |layer: Div, id: &'static str, shown_when_hovered: bool| -> AnyElement {
        if duration.is_zero() {
            return layer
                .opacity(opacity(target, shown_when_hovered))
                .into_any_element();
        }
        layer
            .with_animation(
                (id, changes),
                Animation::new(duration).with_easing(ease_in_out),
                move |layer, delta| {
                    layer.opacity(opacity(from + (target - from) * delta, shown_when_hovered))
                },
            )
            .into_any_element()
    };
    let row = || {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .gap(rpx(12.))
            .px(rpx(8.))
    };
    let stand_in = row().child(theme::placeholder_avatar(32., None, cx)).child(
        div()
            .w(rpx(111.))
            .h(rpx(14.))
            .rounded(rpx(8.))
            .bg(rgb(0x706F74))
            .opacity(0.5),
    );
    let name = own_name(app);
    let name_style = shown.name_style.as_ref().map(shown_name_style);
    let member = row()
        .child(theme::preview_avatar(
            user,
            user_id,
            32.,
            None,
            shown.avatar.clone(),
            shown.avatar_file.clone(),
            decoration,
            cx,
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(rpx(4.))
                .child(
                    div()
                        .min_w_0()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(display_name::render(
                            "styles-nameplate-name",
                            &name,
                            name_style.as_ref(),
                            Surface::Account { hovered },
                            cx,
                        )),
                )
                .children(
                    user.and_then(|user| user.tag.as_ref())
                        .map(|tag| theme::guild_tag(tag, cx)),
                ),
        )
        .when(has_nameplate, |this| {
            this.child(
                div()
                    .id("styles-nameplate-remove")
                    .relative()
                    .size(rpx(24.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(rpx(6.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgba(0x0000007A)))
                    .on_click(on_app(app, |app, cx| {
                        app.update_style_draft(|draft| draft.nameplate = Some(None), cx)
                    }))
                    .child(theme::icon("trash-2", 16., 0xFFFFFF))
                    .child(theme::hover_tooltip(
                        "styles-nameplate-remove-tooltip",
                        "Remove",
                    )),
            )
        });
    let entity = app.entity.clone();
    tile("styles-nameplate", 208., BAR_TILE_HEIGHT)
        .on_hover(move |hovered, _, cx| {
            let hovered = *hovered;
            entity
                .update(cx, |app, cx| {
                    app.update_styles_ui(|ui| ui.nameplate_hover.set(hovered), cx)
                })
                .ok();
        })
        .on_click(on_app(app, |app, cx| {
            let selected = app
                .profile_styles
                .as_ref()
                .and_then(|editor| self::shown(app, editor).nameplate);
            app.open_style_dialog(
                StyleDialog::Collectible {
                    kind: CollectibleKind::Nameplate,
                    selected,
                },
                cx,
            );
        }))
        .children(plate)
        .child(fade(stand_in, "styles-nameplate-stand-in", false))
        .child(fade(member, "styles-nameplate-member", true))
        .into_any_element()
}

/// Discord's default display name font (gg sans) and plain-color effect.
pub(super) const DEFAULT_FONT: u32 = 11;
pub(crate) const SOLID_EFFECT: u32 = 1;

pub(super) fn banner_image(shown: &Shown, cx: &App) -> Option<AnyElement> {
    if let Some(path) = &shown.banner_file {
        return Some(
            img(path.clone())
                .size_full()
                .rounded(rpx(TILE_RADIUS))
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
        );
    }
    let spec = shown.banner.as_ref()?;
    let path = images::resolve(spec, theme::image_px(TILE * 2.), cx)?;
    // The tile's overflow clip is square, so the picture rounds its own corners.
    Some(
        img(path)
            .size_full()
            .rounded(rpx(TILE_RADIUS))
            .object_fit(ObjectFit::Cover)
            .into_any_element(),
    )
}

/// Records where its parent was last painted, for placing popovers beside it.
pub(crate) fn measure_into(cell: Rc<Cell<Option<Bounds<Pixels>>>>) -> impl IntoElement {
    canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| {})
        .absolute()
        .size_full()
}

/// What a profile shows where it has no banner image: the theme's primary color.
pub(super) fn banner_color(editor: &StylesEditor, shown: &Shown) -> u32 {
    if editor.nitro {
        shown.theme_colors.unwrap_or(DEFAULT_THEME)[0]
    } else {
        shown.accent_color.unwrap_or(DEFAULT_THEME[0])
    }
}

pub(super) fn theme_tile(app: &DiscordApp, editor: &StylesEditor, shown: &Shown) -> AnyElement {
    let swatch = |id: &'static str, slot: ColorSlot, color: u32| {
        let bounds = Rc::new(Cell::new(None));
        div()
            .id(id)
            .relative()
            .size(rpx(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(rpx(8.))
            .cursor_pointer()
            .child(measure_into(bounds.clone()))
            .on_click(on_app(app, move |app, cx| {
                app.open_color_picker(slot, color, bounds.get(), cx)
            }))
            .child(
                div()
                    .size(rpx(23.6))
                    .rounded(rpx(8.))
                    .border(rpx(1.78))
                    .border_color(gpui::white())
                    .bg(rgb(color)),
            )
    };
    let base = div()
        .id("styles-theme")
        .group(TILE_GROUP)
        .relative()
        .w(rpx(TILE))
        .h(rpx(TILE))
        .flex()
        .flex_col()
        .items_center()
        .rounded(rpx(TILE_RADIUS))
        .border_1()
        .border_color(rgba(theme::border_soft()));
    if editor.nitro {
        let [primary, accent] = shown.theme_colors.unwrap_or(DEFAULT_THEME);
        base.justify_between()
            .bg(linear_gradient(
                180.,
                linear_color_stop(rgb(primary), 0.),
                linear_color_stop(rgb(accent), 1.),
            ))
            .child(swatch("styles-primary", ColorSlot::Primary, primary))
            .child(swatch("styles-accent", ColorSlot::Accent, accent))
            .into_any_element()
    } else {
        // Discord only takes a banner color on the main profile.
        let color = shown.accent_color.unwrap_or(DEFAULT_THEME[0]);
        base.justify_center()
            .bg(rgb(color))
            .when(editor.scope == StyleScope::Main, |this| {
                this.child(swatch("styles-banner-color", ColorSlot::Banner, color))
            })
            .into_any_element()
    }
}

/// Discord's starting gradient when no theme is set.
pub(super) const DEFAULT_THEME: [u32; 2] = [0x5865F2, 0x232428];

/// Height over width of profile effect thumbnails, which are all 450×880.
const EFFECT_ASPECT: f32 = 880. / 450.;

/// A miniature profile wearing the effect. With `animate`, it plays the effect's own layers,
/// which are animated, instead of the still thumbnail.
pub(super) fn effect_thumbnail(
    editor: &StylesEditor,
    sku_id: Option<u64>,
    width: f32,
    height: f32,
    animate: bool,
    cx: &App,
) -> AnyElement {
    let item = sku_id.and_then(|sku| collectible_item(editor, sku));
    let pixels = theme::image_px(width.max(height));
    // Effect art is drawn for a whole profile, so it spans the card's width from the top.
    let place = |this: Div| {
        this.absolute()
            .top_0()
            .left_0()
            .w(rpx(width))
            .h(rpx(width * EFFECT_ASPECT))
    };
    let layers = item.filter(|_| animate).and_then(|item| {
        let mut layers: Vec<_> = item.effects.iter().enumerate().collect();
        layers.sort_by_key(|(_, layer)| layer.z_index);
        // Only once every layer has decoded, so they start together.
        layers
            .into_iter()
            .map(|(index, layer)| {
                let spec = ImageSpec::Collectible {
                    url: layer.src.clone(),
                };
                let frames = animated::resolve(&spec, pixels, cx)?;
                Some(
                    place(div()).child(
                        animated::image(frames)
                            .id(SharedString::from(format!(
                                "effect-thumbnail-{}-{index}",
                                sku_id.unwrap_or_default()
                            )))
                            .size_full(),
                    ),
                )
            })
            .collect::<Option<Vec<_>>>()
            .filter(|layers| !layers.is_empty())
    });
    let still = layers.is_none().then(|| {
        item.and_then(|item| {
            item.thumbnail_preview_src
                .clone()
                .or_else(|| item.reduced_motion_src.clone())
        })
        .and_then(|url| images::resolve(&ImageSpec::Collectible { url }, pixels, cx))
        .map(|path| place(div()).child(img(path).size_full()))
    });
    div()
        .relative()
        .w(rpx(width))
        .h(rpx(height))
        .overflow_hidden()
        .rounded(rpx(4.))
        .bg(rgb(theme::base_lowest()))
        .shadow(vec![
            BoxShadow::new(px(0.), px(1.), hsla(0., 0., 0., 0.14)).blur_radius(px(4.)),
        ])
        .child(skeleton(width))
        .children(still.flatten())
        .children(layers.into_iter().flatten())
        .into_any_element()
}

/// A greyed-out miniature profile drawn behind effect and frame thumbnails.
fn skeleton(width: f32) -> Div {
    let line = |w: f32| {
        div()
            .w(rpx(w))
            .h(rpx(3.))
            .rounded(rpx(2.))
            .bg(rgb(0x2E2F34))
    };
    div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .child(div().w_full().h(rpx(width * 0.35)).bg(rgb(0x2E2F34)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(3.))
                .p(rpx(width * 0.1))
                .child(
                    div()
                        .size(rpx(width * 0.3))
                        .mt(rpx(-width * 0.18))
                        .rounded_full()
                        .bg(rgb(0x3F4046)),
                )
                .child(line(width * 0.5))
                .child(line(width * 0.35))
                .child(line(width * 0.6)),
        )
}

pub(super) fn frame_thumbnail(
    editor: &StylesEditor,
    sku_id: Option<u64>,
    width: f32,
    height: f32,
    cx: &App,
) -> AnyElement {
    let frame = sku_id.and_then(|sku| {
        let owned = editor.find_collectible(sku)?;
        let item = owned.product.items.iter().find(|item| item.kind == 3)?;
        Some((owned.product.sku_id, item))
    });
    // Back layers sit behind the card and front layers over it, as on a real profile.
    let layers = |front: bool| -> Vec<AnyElement> {
        frame
            .map(|(product_sku, item)| {
                let scale = width / item.inner_width.unwrap_or(1200).max(1) as f32;
                let overflow_x = item.overflow_horizontal.unwrap_or(0) as f32 * scale;
                let frame_width = width + overflow_x * 2.;
                item.layers
                    .iter()
                    .filter(|layer| (layer.order == "front") == front)
                    .filter(|layer| layer.kind != "border" && layer.kind != "rail")
                    .filter_map(|layer| {
                        let spec = ImageSpec::Collectible {
                            url: profile_frame_layer_url(product_sku, layer.id),
                        };
                        let path = images::resolve(&spec, theme::image_px(frame_width), cx)?;
                        let base = div().absolute().left(rpx(-overflow_x)).w(rpx(frame_width));
                        let base = if layer.anchor == "bottom" {
                            base.bottom(rpx(-(item.overflow_bottom.unwrap_or(0) as f32) * scale))
                        } else {
                            base.top(rpx(-(item.overflow_top.unwrap_or(0) as f32) * scale))
                        };
                        Some(base.child(img(path).w(rpx(frame_width))).into_any_element())
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    div()
        .relative()
        .w(rpx(width))
        .h(rpx(height))
        .children(layers(false))
        .child(
            div()
                .absolute()
                .inset_0()
                .overflow_hidden()
                .rounded(rpx(4.))
                .bg(rgb(theme::base_lowest()))
                .child(skeleton(width)),
        )
        .children(layers(true))
        .into_any_element()
}
