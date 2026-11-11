use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement as _;
use gpui_tokio::Tokio;

use crate::{
    app::{DiscordApp, messages::message_body::label, now_snowflake, titlebar},
    discord::Session,
    quick_switcher::{self, Target},
    theme::{self, rpx},
};

const WIDTH: f32 = 480.;
const LIST_HEIGHT: f32 = 280.;
const ROW_HEIGHT: f32 = 44.;
const CHECK_SIZE: f32 = 20.;
const MAX_TARGETS: usize = 5;
const RESULT_LIMIT: usize = 50;

#[derive(Clone, PartialEq)]
struct Destination {
    guild_id: Option<u64>,
    channel_id: u64,
    name: String,
    context: Option<String>,
}

pub(crate) struct ForwardDialog {
    guild_id: Option<u64>,
    channel_id: u64,
    message_id: u64,
    search: Entity<InputState>,
    note: Entity<InputState>,
    results: Vec<Destination>,
    selected: Vec<Destination>,
    sending: bool,
    error: Option<&'static str>,
    _search_events: Subscription,
}

async fn deliver(
    session: Session,
    source: (Option<u64>, u64, u64),
    targets: Vec<(Option<u64>, u64, u64, u64)>,
    note: String,
) -> Result<(), crate::discord::Error> {
    let (source_guild, source_channel, message_id) = source;
    for (guild, channel, forward_nonce, note_nonce) in targets {
        session
            .forward_message(
                source_guild,
                source_channel,
                message_id,
                guild,
                channel,
                forward_nonce,
            )
            .await?;
        if !note.is_empty() {
            session
                .send_forward_note(guild, channel, &note, note_nonce)
                .await?;
        }
    }
    Ok(())
}

impl DiscordApp {
    pub(crate) fn open_forward_dialog(
        &mut self,
        message_id: u64,
        channel_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_lightbox(cx);
        self.message_menu = None;
        let guild_id = (!self.is_home()).then_some(self.selected_guild_id);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let note =
            cx.new(|cx| InputState::new(window, cx).placeholder("Add an optional message..."));
        let events = cx.subscribe_in(&search, window, |app, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                app.rebuild_forward_results(cx);
            }
        });
        search.update(cx, |input, cx| input.focus(window, cx));
        self.menus.forward = Some(ForwardDialog {
            guild_id,
            channel_id,
            message_id,
            search,
            note,
            results: Vec::new(),
            selected: Vec::new(),
            sending: false,
            error: None,
            _search_events: events,
        });
        self.rebuild_forward_results(cx);
    }

    pub(crate) fn close_forward_dialog(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.menus.forward.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    fn rebuild_forward_results(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = &self.menus.forward else {
            return;
        };
        let query = dialog.search.read(cx).value().to_string();
        let term = query.trim().to_lowercase();
        let mut results: Vec<Destination> = self
            .store
            .dm_list()
            .into_iter()
            .map(|dm| dm.channel.id)
            .filter(|id| self.store.can_send(*id))
            .map(|id| Destination {
                guild_id: None,
                channel_id: id,
                name: self.store.private_channel_name(id),
                context: None,
            })
            .filter(|destination| destination.name.to_lowercase().contains(&term))
            .take(RESULT_LIMIT)
            .collect();
        let channels = quick_switcher::search(
            &self.store.guilds,
            &query,
            &self.recent_channels,
            self.selected_guild_id,
            0,
            RESULT_LIMIT,
        );
        let channels = channels.iter().flat_map(|section| &section.items);
        let channels = channels.filter_map(|item| {
            let Target::Channel {
                guild_id,
                channel_id,
            } = item.target
            else {
                return None;
            };
            self.store.can_send(channel_id).then(|| Destination {
                guild_id: Some(guild_id),
                channel_id,
                name: item.name.clone(),
                context: item.guild_name.clone(),
            })
        });
        let channels: Vec<_> = channels.collect();
        results.splice(0..0, channels);
        if let Some(dialog) = &mut self.menus.forward {
            dialog.results = results;
        }
        cx.notify();
    }

    fn toggle_forward_target(&mut self, destination: Destination, cx: &mut Context<Self>) {
        let Some(dialog) = &mut self.menus.forward else {
            return;
        };
        match dialog
            .selected
            .iter()
            .position(|selected| selected.channel_id == destination.channel_id)
        {
            Some(index) => {
                dialog.selected.remove(index);
            }
            None if dialog.selected.len() < MAX_TARGETS => dialog.selected.push(destination),
            None => {}
        }
        cx.notify();
    }

    fn send_forward(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(dialog) = self
            .menus
            .forward
            .as_ref()
            .filter(|dialog| !dialog.sending && !dialog.selected.is_empty())
        else {
            return;
        };
        let source = (dialog.guild_id, dialog.channel_id, dialog.message_id);
        let note = dialog.note.read(cx).value().trim().to_owned();
        let selected: Vec<_> = dialog
            .selected
            .iter()
            .map(|target| (target.guild_id, target.channel_id))
            .collect();
        let targets = selected
            .into_iter()
            .map(|(guild, channel)| {
                let forward_nonce = self.next_nonce();
                (guild, channel, forward_nonce, self.next_nonce())
            })
            .collect();
        if let Some(dialog) = &mut self.menus.forward {
            dialog.sending = true;
            dialog.error = None;
        }
        let request = Tokio::spawn(cx, deliver(session, source, targets, note));
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| match result {
                Ok(Ok(())) => {
                    app.close_forward_dialog(cx);
                }
                failure => {
                    if let Ok(Err(error)) = failure {
                        log::warn!("Could not forward the message: {error}");
                    }
                    if let Some(dialog) = &mut app.menus.forward {
                        dialog.sending = false;
                        dialog.error = Some("Couldn't forward. It may have reached some of them.");
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn next_nonce(&mut self) -> u64 {
        self.compose.last_nonce = now_snowflake().max(self.compose.last_nonce + 1);
        self.compose.last_nonce
    }
}

fn destination_row(
    app: &DiscordApp,
    index: usize,
    destination: &Destination,
    checked: bool,
) -> impl IntoElement + use<> {
    let toggle = app.entity.clone();
    let target = destination.clone();
    div()
        .id(("forward-target", index))
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(12.))
        .h(rpx(ROW_HEIGHT))
        .px(rpx(8.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::hover())))
        .on_click(move |_, _, cx| {
            let target = target.clone();
            toggle
                .update(cx, |app, cx| app.toggle_forward_target(target, cx))
                .ok();
        })
        .child(theme::icon(
            if destination.guild_id.is_some() {
                "hash"
            } else {
                "user"
            },
            20.,
            theme::muted(),
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .child(label(
                            ("forward-name", destination.channel_id),
                            &destination.name,
                        )),
                )
                .children(destination.context.as_ref().map(|context| {
                    div()
                        .truncate()
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::muted()))
                        .child(label(("forward-context", destination.channel_id), context))
                })),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(CHECK_SIZE))
                .rounded(rpx(6.))
                .border_1()
                .border_color(rgba(theme::border_strong()))
                .when(checked, |this| {
                    this.bg(rgb(theme::BLURPLE))
                        .border_color(rgb(theme::BLURPLE))
                        .child(theme::icon("check", 14., 0xFFFFFF))
                }),
        )
}

fn footer_button(id: &'static str, text: String, primary: bool, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .h(rpx(36.))
        .px(rpx(16.))
        .rounded(rpx(8.))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(if primary {
            white()
        } else {
            rgb(theme::bright()).into()
        })
        .bg(rgb(if primary {
            theme::BLURPLE
        } else {
            theme::surface_highest()
        }))
        .when(enabled, |this| {
            this.cursor_pointer().hover(|style| style.opacity(0.85))
        })
        .when(!enabled, |this| this.opacity(0.5))
        .child(text)
}

pub(super) fn render(app: &DiscordApp) -> Option<AnyElement> {
    let dialog = app.menus.forward.as_ref()?;
    let backdrop = app.entity.clone();
    let cancel = app.entity.clone();
    let send = app.entity.clone();
    let rows = dialog.results.iter().enumerate().map(|(index, result)| {
        let checked = dialog
            .selected
            .iter()
            .any(|selected| selected.channel_id == result.channel_id);
        destination_row(app, index, result, checked)
    });
    let can_send = !dialog.selected.is_empty() && !dialog.sending && app.live;
    let send_label = match dialog.selected.len() {
        0 | 1 => "Send".to_owned(),
        count => format!("Send ({count})"),
    };
    Some(
        div()
            .id("forward-backdrop")
            .absolute()
            .inset_0()
            .top(rpx(titlebar::HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .bg(rgba(theme::SCRIM))
            .on_click(move |_, _, cx| {
                backdrop
                    .update(cx, |app, cx| app.close_forward_dialog(cx))
                    .ok();
            })
            .child(theme::selection_guard())
            .child(
                div()
                    .id("forward-dialog")
                    .flex()
                    .flex_col()
                    .gap(rpx(12.))
                    .w(rpx(WIDTH))
                    .max_w_full()
                    .p(rpx(24.))
                    .rounded(rpx(12.))
                    .bg(rgb(theme::elevated()))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .occlude()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .text_size(rpx(20.))
                            .line_height(rpx(24.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(theme::strong()))
                            .child("Forward To"),
                    )
                    .child(
                        div()
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::muted()))
                            .child(format!(
                                "Select up to {MAX_TARGETS} places to share this message."
                            )),
                    )
                    .child(Input::new(&dialog.search))
                    .child(
                        div()
                            .id("forward-results")
                            .flex()
                            .flex_col()
                            .h(rpx(LIST_HEIGHT))
                            .overflow_y_scrollbar()
                            .children(rows),
                    )
                    .child(Input::new(&dialog.note))
                    .children(dialog.error.map(|error| {
                        div()
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::RED))
                            .child(error)
                    }))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(rpx(8.))
                            .child(
                                footer_button("forward-cancel", "Cancel".to_owned(), false, true)
                                    .on_click(move |_, _, cx| {
                                        cancel
                                            .update(cx, |app, cx| app.close_forward_dialog(cx))
                                            .ok();
                                    }),
                            )
                            .child(
                                footer_button("forward-send", send_label, true, can_send).when(
                                    can_send,
                                    |this| {
                                        this.on_click(move |_, _, cx| {
                                            send.update(cx, |app, cx| app.send_forward(cx)).ok();
                                        })
                                    },
                                ),
                            ),
                    ),
            )
            .into_any_element(),
    )
}
