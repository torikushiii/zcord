use super::*;

impl Render for DiscordApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        views::count_render(views::ViewKind::Root);
        messages::rich_text::keep_registered(window, cx);
        theme::set_image_scale(window, cx);
        let streamer = crate::settings::streamer();
        self.store.set_conceal_usernames(streamer.hides_personal());
        if !streamer.enabled {
            self.streamer_notice_dismissed = false;
        }
        self.rpx_per_pixel = theme::BASE_FONT_SIZE / f32::from(window.rem_size());
        self.window_height = f32::from(window.viewport_size().height) * self.rpx_per_pixel;
        if matches!(self.screen, Screen::Chat) {
            self.update_member_subscription();
            self.load_active_now_apps(cx);
        }
        let body = match &self.screen {
            Screen::Login(form) => login::render(form, window, cx).into_any_element(),
            Screen::Connecting => connecting().into_any_element(),
            Screen::Chat => div()
                .relative()
                .flex()
                .flex_1()
                .min_h_0()
                .child(
                    self.views
                        .guild_rail
                        .clone()
                        .cached(views::guild_rail_style()),
                )
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .rounded_tl(rpx(8.))
                        .border_t_1()
                        .border_l_1()
                        .border_color(rgba(theme::border()))
                        .bg(theme::surface(
                            theme::Layer::High,
                            Some(theme::Layer::Higher),
                            theme::base_lower(),
                        ))
                        .child(
                            self.views
                                .channel_list
                                .clone()
                                .cached(views::channel_list_style()),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .bg(theme::surface(
                                    theme::Layer::Chat,
                                    Some(theme::Layer::High),
                                    theme::bg(),
                                ))
                                .when(self.stream_chat_open(), |this| {
                                    this.flex_none()
                                        .ml_auto()
                                        .w(rpx(calls::stream_view::CHAT_WIDTH))
                                })
                                .child(
                                    self.views
                                        .chat_header
                                        .clone()
                                        .cached(views::chat_header_style()),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_1()
                                        .min_h_0()
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .flex_1()
                                                .min_w_0()
                                                .h_full()
                                                .child(
                                                    self.views
                                                        .messages
                                                        .clone()
                                                        .cached(views::messages_style()),
                                                )
                                                .child(self.views.composer.clone().cached(
                                                    views::composer_style(
                                                        self.views.composer_height.get(),
                                                        self.selected_forum().is_some()
                                                            || self.showing_home_page(),
                                                    ),
                                                )),
                                        )
                                        .when(
                                            self.search_results_open()
                                                || (self.show_members
                                                    && (!self.is_home()
                                                        || self.dm_panel_width().is_some())
                                                    && !self.stream_chat_open()
                                                    && !self.thread_pane_open()),
                                            |this| {
                                                this.child(self.views.members.clone().cached(
                                                    views::member_list_style(
                                                        if self.search_results_open() {
                                                            search::RESULTS_WIDTH
                                                        } else {
                                                            self.dm_panel_width().unwrap_or(
                                                                members::member_list::WIDTH,
                                                            )
                                                        },
                                                    ),
                                                ))
                                            },
                                        )
                                        .when(
                                            self.thread_pane_open() && !self.search_results_open(),
                                            |this| {
                                                this.child(self.views.thread_pane.clone().cached(
                                                    views::thread_pane_style(
                                                        self.selected_forum().is_some(),
                                                        self.thread_pane_is_dm(),
                                                    ),
                                                ))
                                            },
                                        ),
                                ),
                        ),
                )
                .when(crate::settings::user_panel_extended(), |this| {
                    this.child(views::user_panel_overlay(&self.views))
                })
                .into_any_element(),
        };
        div()
            .track_focus(&self.focus_handle)
            .on_modifiers_changed(cx.listener(|app, event: &ModifiersChangedEvent, _, cx| {
                if app.shift_held != event.modifiers.shift {
                    app.shift_held = event.modifiers.shift;
                    if app.hover.message.is_some() || app.hover.actions.is_some() {
                        cx.notify();
                    }
                }
            }))
            .on_action(cx.listener(|app, _: &JumpToPresent, window, cx| app.escape(window, cx)))
            .on_scroll_wheel(cx.listener(|app, _, _, cx| {
                app.close_expression_info(cx);
            }))
            .on_action(cx.listener(|app, _: &MarkGuildRead, _, cx| {
                app.mark_guild_read(app.selected_guild_id, cx)
            }))
            .on_action(cx.listener(|app, _: &ComposerUp, window, cx| app.composer_up(window, cx)))
            .on_action(cx.listener(|app, _: &QuickSwitch, window, cx| {
                app.toggle_quick_switcher(window, cx);
            }))
            .on_action(cx.listener(|app, _: &ComposerDown, window, cx| {
                if !(app.move_search(1, cx)
                    || app.move_quick_switcher(1, cx)
                    || app.suggest_input_focused(window, cx)
                        && (app.move_mention_suggestion(1, cx) || app.move_emoji_suggestion(1, cx)))
                {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|app, _: &FocusSearch, window, cx| {
                app.open_search(window, cx);
            }))
            .on_action(cx.listener(|app, _: &ComposerTab, window, cx| {
                if !(app.accept_search(window, cx)
                    || app.suggest_input_focused(window, cx)
                        && (app.accept_mention_suggestion(None, window, cx)
                            || app.accept_emoji_suggestion(None, window, cx)))
                {
                    cx.propagate();
                }
            }))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(rgb(theme::base_lowest()))
            .children(theme::gradient_backdrop())
            .child(smooth_scroll::interceptor())
            .text_color(rgb(theme::text()))
            .text_size(rpx(16.))
            .child(titlebar::render(self, window, cx))
            .when(self.reconnecting, |this| {
                this.child(
                    div()
                        .flex()
                        .justify_center()
                        .py_1()
                        .bg(rgb(theme::YELLOW))
                        .text_size(rpx(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(gpui::black())
                        .child("Connection lost. Reconnecting…"),
                )
            })
            .when(
                crate::settings::streamer().enabled && !self.streamer_notice_dismissed,
                |this| this.child(streamer_bar(cx)),
            )
            .child(body)
            .child(self.views.overlays.clone().cached(views::overlays_style()))
            .when(
                cx.has_active_drag() && self.store.can_send(self.selected_channel_id),
                |this| {
                    this.child(
                        messages::attachments::drop_overlay(self, cx)
                            .opacity(0.)
                            .drag_over::<ExternalPaths>(|style, _, _, _| style.opacity(1.))
                            .on_drop(cx.listener(|app, paths: &ExternalPaths, window, cx| {
                                app.drop_paths(paths.paths().to_vec(), window, cx);
                            })),
                    )
                },
            )
    }
}

pub(super) fn connecting() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .child(theme::logo(64.))
        .child(theme::spinner(24., theme::muted()))
        .child(
            div()
                .text_size(rpx(14.))
                .text_color(rgb(theme::muted()))
                .child("Connecting to Discord…"),
        )
}

fn streamer_bar(cx: &mut Context<DiscordApp>) -> impl IntoElement {
    let streamer = crate::settings::streamer();
    div()
        .relative()
        .flex()
        .flex_none()
        .justify_center()
        .items_center()
        .h(rpx(36.))
        .pl(rpx(4.))
        .pr(rpx(28.))
        .bg(rgb(theme::STREAMER))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(gpui::white())
        .child(if streamer.hides_personal() {
            "Streamer Mode is enabled. Usernames have been truncated."
        } else {
            "Streamer Mode is enabled."
        })
        .child(
            div()
                .id("streamer-mode-off")
                .flex()
                .items_center()
                .h(rpx(24.))
                .ml(rpx(10.))
                .px(rpx(10.))
                .rounded(rpx(3.))
                .border_1()
                .border_color(gpui::white())
                .cursor_pointer()
                .on_click(|_, _, cx| {
                    crate::settings::set_streamer(
                        cx,
                        crate::settings::StreamerMode {
                            enabled: false,
                            ..crate::settings::streamer()
                        },
                    );
                })
                .child("Disable"),
        )
        .child(
            div()
                .id("streamer-mode-dismiss")
                .absolute()
                .top_0()
                .right_0()
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(36.))
                .cursor_pointer()
                .on_click(cx.listener(|app, _, _, cx| {
                    app.streamer_notice_dismissed = true;
                    cx.notify();
                }))
                .child(theme::icon("x", 18., 0xFFFFFF)),
        )
}
