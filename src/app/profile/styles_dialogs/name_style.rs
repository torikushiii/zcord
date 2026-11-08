use super::collectibles::item_size;
use super::*;

/// Display name fonts in the dialog's order, by Discord id.
const FONTS: [(&str, u32); 12] = [
    ("gg sans", 11),
    ("Tempo", 12),
    ("Sakura", 3),
    ("Jellybean", 4),
    ("Modern", 6),
    ("Medieval", 7),
    ("8Bit", 8),
    ("Vampyre", 10),
    ("Monkey Bars", 13),
    ("Mainframe", 14),
    ("Headbang", 15),
    ("Journal", 16),
];
/// Discord's stand-in profile, drawn behind effect and frame grid items.
const SAMPLE_PROFILE: &str = "https://cdn.discordapp.com/assets/content/a859611882903f1102a796c08c68278917821af3e87eb9191df74bf78426971d.png";

fn sample_profile(width: f32, cx: &App) -> Option<Img> {
    let spec = ImageSpec::Collectible {
        url: SAMPLE_PROFILE.to_owned(),
    };
    images::resolve(&spec, theme::image_px(width), cx)
        .map(|path| img(path).object_fit(ObjectFit::Cover))
}

/// An effect over the sample profile, filling its grid item. Hovered, it plays the effect's
/// layers rather than the still thumbnail.
pub(super) fn effect_grid_art(
    editor: &StylesEditor,
    sku_id: u64,
    animate: bool,
    cx: &App,
) -> AnyElement {
    let [width, height] = item_size(CollectibleKind::Effect);
    let pixels = theme::image_px(width.max(height));
    let fill = |this: Div| this.absolute().inset_0().rounded(rpx(4.)).overflow_hidden();
    let item = super::styles_panel::collectible_item(editor, sku_id);
    let layers = item.filter(|_| animate).and_then(|item| {
        item.effects
            .iter()
            .enumerate()
            .map(|(index, layer)| {
                let spec = ImageSpec::Collectible {
                    url: layer.src.clone(),
                };
                let frames = animated::resolve(&spec, pixels, cx)?;
                Some(
                    fill(div()).child(
                        animated::image(frames)
                            .object_fit(ObjectFit::Cover)
                            .size_full()
                            .rounded(rpx(4.))
                            .id(SharedString::from(format!("effect-item-{sku_id}-{index}"))),
                    ),
                )
            })
            .collect::<Option<Vec<_>>>()
            .filter(|layers| !layers.is_empty())
    });
    let still = layers.is_none().then(|| {
        item.and_then(|item| item.thumbnail_preview_src.clone())
            .and_then(|url| images::resolve(&ImageSpec::Collectible { url }, pixels, cx))
            .map(|path| {
                fill(div()).child(
                    img(path)
                        .size_full()
                        .rounded(rpx(4.))
                        .object_fit(ObjectFit::Cover),
                )
            })
    });
    div()
        .absolute()
        .inset_0()
        .children(
            sample_profile(width, cx)
                .map(|image| fill(div()).child(image.size_full().rounded(rpx(4.)))),
        )
        .children(still.flatten())
        .children(layers.into_iter().flatten())
        .into_any_element()
}

pub(super) fn frame_grid_art(editor: &StylesEditor, sku_id: u64, cx: &App) -> AnyElement {
    const WIDTH: f32 = 50.;
    const HEIGHT: f32 = 68.;
    let frame = editor
        .find_collectible(sku_id)
        .and_then(|owned| super::frame::frame_item(&owned.product));
    let overflow = frame
        .map(|(_, item)| super::frame::item_overflow(item, WIDTH))
        .unwrap_or_default();
    let card = div()
        .w(rpx(WIDTH))
        .h(rpx((HEIGHT - overflow.top - overflow.bottom).max(32.)))
        .rounded(rpx(4.))
        .border_1()
        .border_color(rgba(theme::border()))
        .bg(rgb(theme::elevated()))
        .overflow_hidden()
        .children(sample_profile(WIDTH, cx).map(|image| image.size_full().rounded(rpx(4.))))
        .into_any_element();
    div()
        .pt(rpx(overflow.top))
        .pb(rpx(overflow.bottom))
        .child(super::frame::framed_with(frame, WIDTH, card, false, cx))
        .into_any_element()
}

fn set_name_style(
    app: &DiscordApp,
    style: NameStyle,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    on_app(app, move |app, cx| {
        let style = style.clone();
        app.update_styles_ui(|ui| ui.dialog = Some(StyleDialog::NameStyle(style)), cx)
    })
}

/// A font or effect tile: Discord's 4-column grid in the 384px options column.
const NAME_TILE: [f32; 2] = [90., 68.];
const NEW_FONTS: [u32; 2] = [15, 16];

/// Discord's own colors on each effect's tile, whatever colors the user picked.
fn effect_sample(effect_id: u32) -> Vec<u32> {
    match effect_id {
        SOLID_EFFECT => vec![0xD4D5D8],
        2 => vec![0x2AAEA6, 0xFFC490],
        3 => vec![0x7935EF],
        4 => vec![0xF42098],
        5 => vec![0x0FCF86],
        8 => GUMMY_PRESETS[0].to_vec(),
        _ => PRISM_PRESETS[0].to_vec(),
    }
}

fn option_tile(id: ElementId, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex_none()
        .w(rpx(NAME_TILE[0]))
        .h(rpx(NAME_TILE[1]))
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .rounded(rpx(8.))
        .border_1()
        .border_color(if selected {
            rgb(theme::BLURPLE)
        } else {
            rgba(0x97979F1F)
        })
        .bg(rgba(0x97979F14))
        .shadow(vec![
            BoxShadow::new(px(0.), px(1.), hsla(0., 0., 0., 0.14)).blur_radius(px(4.)),
        ])
        .cursor_pointer()
        .hover(|style| style.bg(rgba(0x97979F1F)))
}

fn selector_heading(title: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rpx(6.))
        .mb(rpx(8.))
        .text_size(rpx(16.))
        .line_height(rpx(20.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::strong()))
        .child(title)
        .child(theme::icon("nitro", 16., theme::strong()))
}

pub(super) fn name_style_dialog(
    app: &DiscordApp,
    editor: &StylesEditor,
    style: &NameStyle,
    cx: &App,
) -> AnyElement {
    let fonts = FONTS.iter().map(|&(font, font_id)| {
        let sample = ShownNameStyle {
            font_id: Some(font_id),
            effect_id: None,
            colors: Vec::new(),
        };
        let selected = style.font_id == font_id;
        option_tile(("name-font", font_id as u64).into(), selected)
            .on_click(set_name_style(
                app,
                NameStyle {
                    font_id,
                    ..style.clone()
                },
            ))
            .child(
                div()
                    .text_size(rpx(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(if selected {
                        theme::strong()
                    } else {
                        theme::text()
                    }))
                    .child(display_name::render(
                        ("name-font-sample", font_id as u64),
                        "Gg",
                        Some(&sample),
                        Surface::FontOnly,
                        cx,
                    )),
            )
            .when(NEW_FONTS.contains(&font_id), |this| {
                this.child(
                    div()
                        .absolute()
                        .top(rpx(4.))
                        .right(rpx(4.))
                        .size(rpx(4.))
                        .rounded_full()
                        .bg(rgb(theme::BLURPLE))
                        .shadow(vec![
                            BoxShadow::new(px(0.), px(0.), rgb(theme::BLURPLE).into())
                                .blur_radius(px(4.)),
                        ]),
                )
            })
            .child(theme::hover_tooltip(
                ("name-font-tooltip", font_id as usize),
                font,
            ))
            .into_any_element()
    });
    let effects = EFFECTS.iter().map(|&(label, effect_id)| {
        let sample = ShownNameStyle {
            font_id: None,
            effect_id: Some(effect_id),
            colors: effect_sample(effect_id),
        };
        option_tile(
            ("name-effect", effect_id as u64).into(),
            style.effect_id == effect_id,
        )
        .on_click(set_name_style(app, with_effect(style, effect_id)))
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .child(display_name::render(
                    ("name-effect-sample", effect_id as u64),
                    label,
                    Some(&sample),
                    Surface::Account { hovered: true },
                    cx,
                )),
        )
        .into_any_element()
    });
    let swatch_border = |selected: bool| {
        if selected {
            rgb(theme::BLURPLE)
        } else {
            rgba(0)
        }
    };
    let presets = presets(style.effect_id);
    let custom_selected = !style.colors.is_empty()
        && !presets
            .iter()
            .any(|preset| with_preset(style, preset).colors == style.colors);
    let swatches = presets.iter().enumerate().map(|(index, preset)| {
        let selected = with_preset(style, preset).colors == style.colors;
        div()
            .id(("name-color", index))
            .flex_none()
            .size(rpx(40.))
            .flex()
            .overflow_hidden()
            .rounded(rpx(8.))
            .border_2()
            .border_color(swatch_border(selected))
            .cursor_pointer()
            .on_click(set_name_style(app, with_preset(style, preset)))
            .child(color_fill(preset, blends(style.effect_id), 6.))
            .into_any_element()
    });
    let custom = if blends(style.effect_id) {
        custom_gradient(app, style, custom_selected)
    } else {
        let color = style.colors.first().copied().unwrap_or(GUMMY_PRESETS[2][0]);
        let bounds = Rc::new(Cell::new(None));
        div()
            .id("name-color-custom")
            .relative()
            .flex_none()
            .size(rpx(40.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(rpx(8.))
            .map(|this| {
                if custom_selected {
                    this.border_2()
                        .border_color(rgb(theme::BLURPLE))
                        .bg(rgb(color))
                } else {
                    this.border_1()
                        .border_color(rgba(0x97979F3D))
                        .bg(rgba(0x97979F1F))
                }
            })
            .cursor_pointer()
            .child(measure_into(bounds.clone()))
            .on_click(on_app(app, move |app, cx| {
                app.open_color_picker(ColorSlot::Name(0), color, bounds.get(), cx)
            }))
            .child(theme::icon("pipette", 14., 0xFFFFFF))
            .into_any_element()
    };
    let colors = div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .child(custom)
        .children(swatches);
    let grid = |tiles: Vec<AnyElement>| {
        div()
            .flex()
            .flex_wrap()
            .gap(rpx(8.))
            .w(rpx(384.))
            .children(tiles)
    };
    let section = |title: &'static str, content: Div| {
        div()
            .flex()
            .flex_col()
            .mt(rpx(24.))
            .child(selector_heading(title))
            .child(content)
    };
    let options = div()
        .id("name-style-options")
        .flex()
        .flex_col()
        .flex_none()
        .w(rpx(424.))
        .h_full()
        .overflow_y_scroll()
        .pt(rpx(24.))
        .pb(rpx(24.))
        .pl(rpx(24.))
        .pr(rpx(16.))
        .child(
            div()
                .mb(rpx(8.))
                .text_size(rpx(20.))
                .line_height(rpx(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Change Display Name Style"),
        )
        .child(section("Choose Font", grid(fonts.collect())))
        .child(section("Choose Effect", grid(effects.collect())))
        .child(section("Choose Color", colors));
    let body = div()
        .flex()
        .min_h_0()
        .h(rpx(631.))
        .child(options)
        .child(name_style_preview(app, editor, style, cx))
        .into_any_element();
    let footer = div()
        .border_t_1()
        .border_color(rgba(theme::border_subtle()))
        .child(
            button("name-style-apply", "Apply", true, 139.)
                .on_click(on_app(app, |app, cx| app.apply_style_dialog(cx))),
        )
        .child({
            let style = style.clone();
            button("name-style-surprise", "Surprise Me", false, 139.)
                .child(theme::icon("dices", 16., theme::strong()))
                .on_click(on_app(app, move |app, cx| {
                    let next = surprise(&style);
                    app.update_styles_ui(|ui| ui.dialog = Some(StyleDialog::NameStyle(next)), cx)
                }))
        });
    shell(app, editor, None, 836., body, footer)
}

/// A random font, effect and preset, never the current combination.
fn surprise(current: &NameStyle) -> NameStyle {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos() as usize);
    let font = FONTS[seed % FONTS.len()].1;
    let effect = EFFECTS[(seed / FONTS.len()) % EFFECTS.len()].1;
    let presets = presets(effect);
    let preset = presets[(seed / 97) % presets.len()];
    let next = with_preset(
        &NameStyle {
            font_id: font,
            effect_id: effect,
            colors: Vec::new(),
        },
        preset,
    );
    if next == *current {
        with_effect(
            &next,
            if effect == SOLID_EFFECT {
                2
            } else {
                SOLID_EFFECT
            },
        )
    } else {
        next
    }
}

const NAME_PREVIEW_WIDTH: f32 = 410.;
/// Discord's preview: the profile card, a chat message and a member row overlapping down the
/// right of the dialog, over the profile's theme and a faded banner, with a toggle that shows
/// the message and member row in light mode.
fn name_style_preview(
    app: &DiscordApp,
    editor: &StylesEditor,
    style: &NameStyle,
    cx: &App,
) -> AnyElement {
    let user_id = app.store.current_user_id;
    let user = app.store.user(user_id);
    let current = shown(app, editor);
    let name = own_name(app);
    let shown_style = shown_name_style(style);
    let light = editor.ui.name_preview_light;
    let decoration = current
        .decoration
        .and_then(|sku| decoration_of(app, editor, sku));
    let avatar = |size: f32, decorated: bool| {
        theme::preview_avatar(
            user,
            user_id,
            size,
            None,
            current.avatar.clone(),
            current.avatar_file.clone(),
            decoration.as_ref().filter(|_| decorated),
            cx,
        )
    };
    let card = app.profile.as_ref();
    let backdrop = card
        .and_then(|card| card.theme())
        .map_or_else(|| rgb(theme::bg()).into(), |theme| theme.modal_background());
    let banner = current
        .banner_file
        .clone()
        .map(|path| ImageSpec::LocalGif { path })
        .or_else(|| current.banner.clone())
        .and_then(|spec| super::banner_backdrop(&spec, NAME_PREVIEW_WIDTH, 0., cx));
    let frame = current
        .frame
        .and_then(|sku| editor.find_collectible(sku))
        .map(|owned| owned.product.clone());
    let profile = card.map(|card| {
        super::popout::style_preview(
            app,
            card,
            None,
            frame.as_deref().and_then(super::frame::frame_item),
            &editor.ui.preview_effect.1,
            true,
            cx,
        )
    });
    let (surface, border, text, muted) = if light {
        (0xFFFFFF, rgba(0x0000001F), 0x313338, 0x5C5E66)
    } else {
        (
            theme::surface_high(),
            rgba(theme::border_subtle()),
            theme::text(),
            theme::muted(),
        )
    };
    let now = chrono::Local::now().timestamp();
    let message = div()
        .relative()
        .ml(rpx(25.))
        .mt(rpx(-10.))
        .mb(rpx(-5.))
        .min_w(rpx(320.))
        .py(rpx(20.))
        .pl(rpx(70.))
        .pr(rpx(20.))
        .rounded(rpx(8.))
        .border_1()
        .border_color(border)
        .bg(rgb(surface))
        .child(
            div()
                .absolute()
                .left(rpx(16.))
                .top(rpx(22.))
                .child(avatar(40., true)),
        )
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(rpx(4.))
                .child(
                    div()
                        .max_w(rpx(180.))
                        .text_size(rpx(16.))
                        .line_height(rpx(22.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(if light { 0x060607 } else { theme::strong() }))
                        .child(display_name::render(
                            "name-style-preview-message",
                            &name,
                            Some(&shown_style),
                            Surface::Account { hovered: true },
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(rpx(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(muted))
                        .child(crate::text::timestamp::format_timestamp(now, 't')),
                ),
        )
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(22.))
                .text_color(rgb(text))
                .child("does anyone read this?"),
        );
    let plate = current
        .nameplate
        .and_then(|sku| nameplate_of(app, editor, sku))
        .and_then(|plate| member_nameplate_layer(&plate, user_id, 42., true, 8., cx));
    let member = div()
        .relative()
        .w(rpx(280.))
        .h(rpx(44.))
        .overflow_hidden()
        .rounded(rpx(8.))
        .border_1()
        .border_color(border)
        .bg(rgb(if light {
            0xEBEBED
        } else {
            theme::surface_highest()
        }))
        .children(plate)
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .gap(rpx(12.))
                .pl(rpx(8.))
                .pr(rpx(32.))
                .child(avatar(32., true))
                .child(
                    div()
                        .min_w_0()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(if light { 0x060607 } else { theme::strong() }))
                        .child(display_name::render(
                            "name-style-preview-row",
                            &name,
                            Some(&shown_style),
                            Surface::Account { hovered: true },
                            cx,
                        )),
                )
                .children(
                    user.and_then(|user| user.tag.as_ref())
                        .map(|tag| theme::guild_tag(tag, cx)),
                ),
        );
    let toggle_option = |id: &'static str, icon: &'static str, selected: bool| {
        div()
            .id(id)
            .size(rpx(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(rpx(8.))
            .cursor_pointer()
            .map(|this| {
                if selected {
                    this.border_1()
                        .border_color(rgba(0x97979F29))
                        .bg(rgb(theme::surface_high()))
                } else {
                    this.hover(|style| style.bg(rgba(0x97979F1F)))
                }
            })
            .on_click(on_app(app, move |app, cx| {
                let light = icon == "theme-light";
                app.update_styles_ui(|ui| ui.name_preview_light = light, cx)
            }))
            .child(theme::icon(
                icon,
                16.,
                if selected { 0xDCDCDF } else { 0x96979E },
            ))
    };
    // `min_w_0` and the clip hold it to its column; otherwise the note's unwrapped
    // width and the frame art stretch it past the dialog's edge.
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_hidden()
        .p(rpx(12.))
        .bg(gpui::black())
        .child(div().absolute().inset_0().bg(backdrop))
        .children(banner)
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .justify_center()
                .mx_auto()
                .mt(rpx(32.))
                .pl(rpx(16.))
                .children(profile.map(|profile| div().w(rpx(300.)).child(profile)))
                .child(message)
                .child(member),
        )
        .child(
            div()
                .relative()
                .flex()
                .items_center()
                .justify_end()
                .px(rpx(16.))
                .pb(rpx(16.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .text_color(rgb(theme::muted()))
                        .child(
                            "Note: Display name colors and effects will not appear in servers. \
                             Styles may also look different across Discord and light/dark mode.",
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .gap(rpx(4.))
                        .ml(rpx(16.))
                        .p(rpx(4.))
                        .rounded(rpx(12.))
                        .bg(gpui::black())
                        .child(toggle_option("name-preview-dark", "theme-dark", !light))
                        .child(toggle_option("name-preview-light", "theme-light", light)),
                ),
        )
        .into_any_element()
}
