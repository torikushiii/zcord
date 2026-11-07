use crate::app::menu::PopupButton as _;
use gpui::{prelude::*, *};
use gpui_component::input::Input;

use super::{SettingsPage, sections};
use crate::{
    app::{DiscordApp, menu, voice::switch},
    discord::settings_proto::{ActivityPrivacy, NewServerSharing},
    media::images,
    store::{Guild, RailEntry},
    theme::{self, rpx},
};

const FOLDED_AFTER: usize = 10;
const SERVER_ICON: f32 = 48.;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ServerSort {
    #[default]
    ServerOrder,
    Newest,
    SharingOn,
    SharingOff,
}

impl ServerSort {
    const ALL: [Self; 4] = [
        Self::ServerOrder,
        Self::Newest,
        Self::SharingOn,
        Self::SharingOff,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::ServerOrder => "Server Order",
            Self::Newest => "Newest",
            Self::SharingOn => "Activity Sharing On",
            Self::SharingOff => "Activity Sharing Off",
        }
    }
}

impl DiscordApp {
    fn set_server_sort(&mut self, sort: Option<ServerSort>, cx: &mut Context<Self>) {
        if let Some(settings) = &mut self.settings_page {
            if let Some(sort) = sort {
                settings.server_sort = sort;
            }
            settings.server_sort_open = false;
            cx.notify();
        }
    }

    fn toggle_server_sort_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(settings) = &mut self.settings_page {
            settings.server_sort_open ^= true;
            cx.notify();
        }
    }

    fn toggle_servers_expanded(&mut self, cx: &mut Context<Self>) {
        if let Some(settings) = &mut self.settings_page {
            settings.servers_expanded ^= true;
            cx.notify();
        }
    }
}

pub(super) fn page(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> Vec<AnyElement> {
    let privacy = app.activity_privacy();
    let toggle = |id: &'static str,
                  label: &'static str,
                  description: &'static str,
                  on: bool,
                  change: fn(&mut ActivityPrivacy)| {
        let entity = app.entity.clone();
        sections::toggle_row(app, settings, id, label, description, on, move |cx, _| {
            entity
                .update(cx, |app, cx| app.change_activity_privacy(change, cx))
                .ok();
        })
    };
    let mut shared = vec![toggle(
        "settings-share-activity",
        "Share my activity",
        "Share activity information from games and connected apps I use, including when and \
         how I engage.",
        privacy.share_activity,
        |privacy| privacy.share_activity ^= true,
    )];
    shared.push(toggle(
        "settings-share-come-online",
        "Share when I come online",
        "Allow friends to receive a push notification when you come online.",
        privacy.share_come_online,
        |privacy| privacy.share_come_online ^= true,
    ));
    shared.push(toggle(
        "settings-share-go-live",
        "Share when I start streaming",
        "Allow members that you interact with the most to get a notification when you start \
         streaming in small servers.",
        privacy.share_go_live,
        |privacy| privacy.share_go_live ^= true,
    ));
    let joining = vec![
        sections::note(
            "Some games allow users to join the game you're playing when viewing your activity.",
        ),
        toggle(
            "settings-join-friends",
            "Allow friends to join my game",
            "Allows friends to join without sending a request.",
            privacy.join_friends,
            |privacy| privacy.join_friends ^= true,
        ),
        toggle(
            "settings-join-voice",
            "Allow voice channel participants to join my game",
            "Allow anyone in a shared voice channel to join my game without sending a request. \
             This feature only works in non-Community enabled servers.",
            privacy.join_voice,
            |privacy| privacy.join_voice ^= true,
        ),
    ];
    vec![
        sections::section("What Activity I Share", shared),
        sections::section(
            "Where I Share Activity",
            vec![
                new_servers(app, privacy.new_servers),
                servers(app, settings, &privacy, cx),
            ],
        ),
        sections::section("Activity Joining", joining),
    ]
}

fn new_servers(app: &DiscordApp, current: NewServerSharing) -> AnyElement {
    let option = |id: &'static str, label: &'static str, value: NewServerSharing| {
        let entity = app.entity.clone();
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(rpx(12.))
            .h(rpx(36.))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| {
                        app.change_activity_privacy(|privacy| privacy.new_servers = value, cx)
                    })
                    .ok();
            })
            .child(menu::radio(current == value))
            .child(
                div()
                    .text_size(rpx(16.))
                    .line_height(rpx(20.))
                    .text_color(rgb(theme::text()))
                    .child(label),
            )
    };
    div()
        .flex()
        .flex_col()
        .py(rpx(12.))
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::strong()))
                .child("Automatically share my activity when joining a server"),
        )
        .child(
            div()
                .mt(rpx(4.))
                .mb(rpx(6.))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::muted()))
                .child("Your activity is always shared with your friends."),
        )
        .child(option(
            "settings-new-servers-all",
            "Share in all servers",
            NewServerSharing::All,
        ))
        .child(option(
            "settings-new-servers-small",
            "Share in servers with 200 or fewer members",
            NewServerSharing::SmallOnly,
        ))
        .child(option(
            "settings-new-servers-none",
            "Do not share in any servers",
            NewServerSharing::Nowhere,
        ))
        .into_any_element()
}

fn servers(
    app: &DiscordApp,
    settings: &SettingsPage,
    privacy: &ActivityPrivacy,
    cx: &App,
) -> AnyElement {
    let query = settings
        .server_search
        .read(cx)
        .value()
        .trim()
        .to_lowercase();
    let shared = |guild: &Guild| !privacy.hidden_guild_ids.contains(&guild.id);
    let mut guilds: Vec<&Guild> = app
        .store
        .rail
        .iter()
        .flat_map(|entry| match entry {
            RailEntry::Guild(id) => std::slice::from_ref(id),
            RailEntry::Folder(folder) => folder.guild_ids.as_slice(),
        })
        .filter_map(|id| app.store.guilds.iter().find(|guild| guild.id == *id))
        .filter(|guild| query.is_empty() || guild.name.to_lowercase().contains(&query))
        .collect();
    match settings.server_sort {
        ServerSort::ServerOrder => {}
        ServerSort::Newest => guilds.sort_by(|left, right| {
            let joined = |guild: &Guild| app.store.guild_membership(guild.id).1.map(str::to_owned);
            joined(right).cmp(&joined(left))
        }),
        ServerSort::SharingOn => guilds.sort_by_key(|guild| !shared(guild)),
        ServerSort::SharingOff => guilds.sort_by_key(|guild| shared(guild)),
    }
    let folded = query.is_empty() && guilds.len() > FOLDED_AFTER + 1;
    let (shown, rest) = if folded && !settings.servers_expanded {
        guilds.split_at(FOLDED_AFTER)
    } else {
        (guilds.as_slice(), &[][..])
    };
    let all_on = app.store.guilds.iter().all(shared);
    let toggle_all = app.entity.clone();
    let all_ids: Vec<u64> = app.store.guilds.iter().map(|guild| guild.id).collect();
    let sort_entity = app.entity.clone();
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .h(rpx(32.))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .child(
            div()
                .relative()
                .child(
                    div()
                        .id("settings-server-sort")
                        .flex()
                        .items_center()
                        .gap(rpx(4.))
                        .cursor_pointer()
                        .text_color(rgb(theme::muted()))
                        .hover(|style| style.text_color(rgb(theme::text())))
                        .on_popup_click(
                            crate::app::menu::shown(app, |app| {
                                app.settings_page
                                    .as_ref()
                                    .is_some_and(|settings| settings.server_sort_open)
                            }),
                            move |_, _, cx| {
                                sort_entity
                                    .update(cx, |app, cx| app.toggle_server_sort_menu(cx))
                                    .ok();
                            },
                        )
                        .child(settings.server_sort.label())
                        .child(theme::icon("chevron-down", 16., theme::muted())),
                )
                .when(settings.server_sort_open, |this| {
                    this.child(sort_menu(app, settings.server_sort))
                }),
        )
        .child(
            div()
                .id("settings-servers-toggle-all")
                .cursor_pointer()
                .text_color(rgb(theme::link()))
                .hover(|style| style.underline())
                .on_click(move |_, _, cx| {
                    let ids = all_ids.clone();
                    toggle_all
                        .update(cx, |app, cx| {
                            app.change_activity_privacy(
                                |privacy| {
                                    privacy.hidden_guild_ids = if all_on { ids } else { Vec::new() }
                                },
                                cx,
                            )
                        })
                        .ok();
                })
                .child(if all_on {
                    "Toggle All Off"
                } else {
                    "Toggle All On"
                }),
        );
    let rows = shown
        .iter()
        .map(|guild| server_row(app, guild, shared(guild), cx));
    let more = folded.then(|| more_row(app, rest, settings.servers_expanded, cx));
    div()
        .flex()
        .flex_col()
        .gap(rpx(16.))
        .pt(rpx(8.))
        .child(search_box(settings))
        .child(header)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(24.))
                .pb(rpx(24.))
                .children(rows)
                .children(more),
        )
        .into_any_element()
}

fn search_box(settings: &SettingsPage) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .h(rpx(40.))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .bg(rgb(theme::input_bg()))
        .border_1()
        .border_color(rgb(theme::input_border()))
        .child(theme::icon("search", 18., theme::text()))
        .child(
            div().flex_1().min_w_0().child(
                Input::new(&settings.server_search)
                    .appearance(false)
                    .aria_label("Search my servers")
                    .px_0()
                    .py_0()
                    .text_size(rpx(16.))
                    .line_height(rpx(20.))
                    .text_color(rgb(theme::text())),
            ),
        )
}

fn sort_menu(app: &DiscordApp, current: ServerSort) -> AnyElement {
    let close = app.entity.clone();
    let items = ServerSort::ALL.into_iter().map(|sort| {
        let entity = app.entity.clone();
        div()
            .id(("settings-server-sort-option", sort as u64))
            .flex()
            .items_center()
            .justify_between()
            .gap(rpx(16.))
            .h(rpx(44.))
            .px(rpx(12.))
            .rounded(rpx(6.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(theme::selected())))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                entity
                    .update(cx, |app, cx| app.set_server_sort(Some(sort), cx))
                    .ok();
            })
            .child(
                div()
                    .text_size(rpx(16.))
                    .line_height(rpx(20.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::strong()))
                    .child(sort.label()),
            )
            .child(menu::radio(sort == current))
    });
    div()
        .absolute()
        .top(rpx(26.))
        .left_0()
        .size_0()
        .child(
            deferred(
                anchored()
                    .anchor(Anchor::TopLeft)
                    .snap_to_window_with_margin(Edges::all(px(8.)))
                    .child(
                        div()
                            .id("settings-server-sort-menu")
                            .flex()
                            .flex_col()
                            .min_w(rpx(224.))
                            .p(rpx(8.))
                            .rounded(rpx(8.))
                            .bg(rgb(theme::elevated()))
                            .border_1()
                            .border_color(rgba(theme::border_bright()))
                            .shadow_lg()
                            .occlude()
                            .on_mouse_down_out(move |_, _, cx| {
                                close
                                    .update(cx, |app, cx| app.set_server_sort(None, cx))
                                    .ok();
                            })
                            .children(items),
                    ),
            )
            .with_priority(3),
        )
        .into_any_element()
}

fn server_icon(guild: &Guild, cx: &App) -> AnyElement {
    let image = guild
        .icon_spec()
        .and_then(|spec| images::resolve(&spec, theme::image_px(SERVER_ICON), cx));
    match image {
        Some(path) => img(path)
            .size(rpx(SERVER_ICON))
            .flex_none()
            .rounded(rpx(16.))
            .into_any_element(),
        None => div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(rpx(SERVER_ICON))
            .rounded(rpx(16.))
            .bg(rgba(theme::control_bg()))
            .text_size(rpx(16.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::text()))
            .child(guild.initials.clone())
            .into_any_element(),
    }
}

fn server_row(app: &DiscordApp, guild: &Guild, on: bool, cx: &App) -> AnyElement {
    let id = guild.id;
    let entity = app.entity.clone();
    let members = app.store.guild_membership(id).0;
    div()
        .flex()
        .items_center()
        .gap(rpx(16.))
        .h(rpx(48.))
        .child(server_icon(guild, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(rpx(4.))
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .child(guild.name.clone()),
                )
                .children(members.map(|count| {
                    div()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::muted()))
                        .child(members_label(count))
                })),
        )
        .child(
            div()
                .id(("settings-server-activity", id))
                .flex_none()
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    entity
                        .update(cx, |app, cx| {
                            app.change_activity_privacy(
                                |privacy| {
                                    if on {
                                        privacy.hidden_guild_ids.push(id);
                                    } else {
                                        privacy.hidden_guild_ids.retain(|hidden| *hidden != id);
                                    }
                                },
                                cx,
                            )
                        })
                        .ok();
                })
                .child(switch("settings-server-activity", on, None)),
        )
        .into_any_element()
}

fn more_row(app: &DiscordApp, rest: &[&Guild], expanded: bool, cx: &App) -> AnyElement {
    let entity = app.entity.clone();
    let (title, subtitle) = if expanded {
        ("Show fewer servers".to_owned(), None)
    } else {
        let names: Vec<&str> = rest
            .iter()
            .take(3)
            .map(|guild| guild.name.as_str())
            .collect();
        let others = rest.len().saturating_sub(names.len());
        let subtitle = if others > 0 {
            format!("{}, and +{others} more servers", names.join(", "))
        } else {
            names.join(", ")
        };
        (format!("Show {} more servers", rest.len()), Some(subtitle))
    };
    let icons = (!expanded).then(|| {
        let mut stack = div().relative().flex_none().size(rpx(SERVER_ICON));
        if let Some(back) = rest.get(1) {
            stack = stack.child(div().absolute().left(rpx(-4.)).child(server_icon(back, cx)));
        }
        if let Some(front) = rest.first() {
            stack = stack.child(
                div()
                    .absolute()
                    .left(rpx(8.))
                    .p(rpx(2.))
                    .m(rpx(-2.))
                    .rounded(rpx(18.))
                    .bg(rgb(theme::bg()))
                    .child(server_icon(front, cx)),
            );
        }
        stack
    });
    div()
        .id("settings-servers-more")
        .flex()
        .items_center()
        .gap(rpx(16.))
        .min_h(rpx(48.))
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.toggle_servers_expanded(cx))
                .ok();
        })
        .children(icons)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(rpx(4.))
                .when(!expanded, |this| this.ml(rpx(8.)))
                .child(
                    div()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::text()))
                        .child(title),
                )
                .children(subtitle.map(|subtitle| {
                    div()
                        .truncate()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::faint()))
                        .child(subtitle)
                })),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(40.))
                .rounded(rpx(8.))
                .bg(rgba(theme::control_bg()))
                .child(theme::icon(
                    if expanded {
                        "chevron-up"
                    } else {
                        "chevron-down"
                    },
                    18.,
                    theme::text(),
                )),
        )
        .into_any_element()
}

fn members_label(count: u32) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!(
        "{grouped} {}",
        if count == 1 { "member" } else { "members" }
    )
}

#[cfg(test)]
mod tests {
    use super::members_label;

    #[test]
    fn member_counts_group_thousands() {
        assert_eq!(members_label(1), "1 member");
        assert_eq!(members_label(3), "3 members");
        assert_eq!(members_label(11_628), "11,628 members");
        assert_eq!(members_label(1_234_567), "1,234,567 members");
    }
}
