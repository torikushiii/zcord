use chrono::{Local, NaiveDate, TimeZone};
use gpui::{prelude::*, *};
use gpui_component::input::{Input, Textarea};
use gpui_component::scroll::ScrollableElement as _;

use crate::{
    app::forum_actions::ForumHeader,
    app::{
        DiscordApp,
        chat::{author_name, body_context},
        display_name::{self, Surface},
        embeds::emoji_element,
        message_body::{label, preview_pieces, render_body},
        rich_text::rich_text,
    },
    discord::ForumSort,
    media::images::{self, ImageSpec},
    store::{ForumPost, ForumTag, Reaction, StatusEmoji, Timestamp},
    text::emoji,
    theme::{self, WithoutTextSelection, rpx},
};

const THUMBNAIL: f32 = 72.;
const GAP: f32 = 16.;
const CARD_MIN_WIDTH: f32 = 360.;
const PREVIEW_HEIGHT: f32 = 293.;
const MOSAIC_GAP: f32 = 4.;
/// Cards show this many tags, then a "+N" chip for the rest.
const VISIBLE_TAGS: usize = 3;
const MENU_WIDTH: f32 = 180.;

pub fn render(app: &DiscordApp, forum_id: u64, cx: &App) -> impl IntoElement + use<> {
    let tags = app.store.forum_tags(forum_id);
    let filter = app.forum_tag_filter(forum_id).to_vec();
    let match_all = app.forum_match_all(forum_id);
    let header = app
        .forum_header
        .as_ref()
        .filter(|header| header.forum_id == forum_id);
    let results = header.and_then(|header| header.results.clone());
    let searching = header.is_some_and(|header| header.searching);
    let posts: Vec<&ForumPost> = app
        .store
        .forum_posts_sorted(forum_id, app.forum_sort(forum_id))
        .into_iter()
        .filter(|post| results.as_ref().is_none_or(|ids| ids.contains(&post.id)))
        .filter(|post| {
            if match_all {
                filter.iter().all(|tag| post.tag_ids.contains(tag))
            } else {
                filter.is_empty() || post.tag_ids.iter().any(|tag| filter.contains(tag))
            }
        })
        .collect();
    let gallery = app.forum_gallery(forum_id);
    let loading = app.forum_loading();
    let more = app.forum_has_more();
    let last_read = app.store.last_read(forum_id);
    let new = |post: &ForumPost| {
        last_read.is_some_and(|read| post.id > read)
            && !post.archived
            && post.owner_id != app.store.current_user_id
    };
    let cards: Vec<AnyElement> = posts
        .iter()
        .map(|post| {
            if gallery {
                gallery_card(app, post, &tags, new(post), cx).into_any_element()
            } else {
                list_card(app, post, &tags, new(post), cx).into_any_element()
            }
        })
        .collect();
    let empty = posts.is_empty();
    let entity = app.entity.clone();
    div()
        .id(("forum", forum_id))
        .flex()
        .flex_col()
        .gap(rpx(GAP))
        .flex_1()
        .min_h_0()
        .w_full()
        .px(rpx(24.))
        .pt(rpx(24.))
        .pb(rpx(16.))
        .overflow_y_scrollbar()
        .children(header.map(|header| header_card(app, header, &tags, cx)))
        .child(toolbar(app, forum_id, &tags, &filter, cx))
        .map(|this| {
            if gallery {
                this.child(gallery_grid(app, cards))
            } else {
                this.child(div().flex().flex_col().gap(rpx(8.)).children(cards))
            }
        })
        .when(empty && !loading && !searching, |this| {
            this.child(note(if results.is_some() {
                "No posts match your search"
            } else if filter.is_empty() {
                "No posts yet"
            } else {
                "No posts match these tags"
            }))
        })
        .when(loading || searching, |this| {
            this.child(note(if searching {
                "Searching…"
            } else {
                "Loading posts…"
            }))
        })
        .when(more && !loading, |this| {
            this.child(
                div()
                    .id("forum-more")
                    .flex()
                    .flex_none()
                    .justify_center()
                    .py(rpx(10.))
                    .rounded(rpx(8.))
                    .text_size(rpx(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::text()))
                    .bg(rgb(theme::elevated()))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgba(theme::hover())))
                    .child("Load more posts")
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |app, cx| app.load_more_forum(cx)).ok();
                    }),
            )
        })
}

fn header_card(
    app: &DiscordApp,
    header: &ForumHeader,
    tags: &[ForumTag],
    cx: &App,
) -> impl IntoElement {
    let start = app.entity.clone();
    let creating = header.creating;
    let primary = |id: &'static str, enabled: bool| {
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .gap(rpx(6.))
            .h(rpx(32.))
            .px(rpx(12.))
            .rounded(rpx(8.))
            .bg(rgb(theme::BLURPLE))
            .border_1()
            .border_color(rgba(theme::border_bright()))
            .when(enabled, |this| {
                this.cursor_pointer().hover(|style| style.opacity(0.9))
            })
            .when(!enabled, |this| this.opacity(0.5))
            .text_size(rpx(14.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(white())
    };
    let title_row = div()
        .flex()
        .items_center()
        .h(rpx(48.))
        .pl(rpx(12.))
        .pr(rpx(8.))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(26.))
                .mr(rpx(8.))
                .child(theme::icon(
                    if creating { "message-circle" } else { "search" },
                    20.,
                    theme::muted(),
                )),
        )
        .child(
            div().flex_1().min_w_0().child(
                Input::new(&header.input)
                    .appearance(false)
                    .px_0()
                    .text_size(rpx(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::strong())),
            ),
        )
        .when(!creating, |this| {
            let blocked = app.new_post_blocked(header.forum_id);
            this.child(
                primary("forum-new-post", blocked.is_none())
                    .relative()
                    .map(|this| match blocked {
                        Some(reason) => {
                            this.child(theme::hover_tooltip("forum-new-post-tip", reason))
                        }
                        None => this.on_click(move |_, window, cx| {
                            start
                                .update(cx, |app, cx| app.start_forum_post(window, cx))
                                .ok();
                        }),
                    })
                    .child(theme::icon("message-circle", 16., 0xFFFFFF))
                    .child("New Post"),
            )
        });
    div()
        .flex()
        .flex_none()
        .flex_col()
        .rounded(rpx(12.))
        .bg(rgba(theme::recessed()))
        .border_1()
        .border_color(rgba(theme::outline()))
        .child(title_row)
        .when(creating, |this| {
            let cancel = app.entity.clone();
            let submit = app.entity.clone();
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rpx(12.))
                    .px(rpx(12.))
                    .pb(rpx(12.))
                    .when(!tags.is_empty(), |this| {
                        let required = app.store.forum_requires_tag(header.forum_id);
                        this.child(
                            div()
                                .text_size(rpx(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::muted()))
                                .child(if required {
                                    "Tags (at least one required)"
                                } else {
                                    "Tags"
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap(rpx(8.))
                                .children(tags.iter().map(|tag| {
                                    let entity = app.entity.clone();
                                    let id = tag.id;
                                    toolbar_button(("new-post-tag", id), header.tags.contains(&id))
                                        .on_click(move |_, _, cx| {
                                            entity
                                                .update(cx, |app, cx| {
                                                    app.toggle_new_post_tag(id, cx)
                                                })
                                                .ok();
                                        })
                                        .children(
                                            tag.emoji
                                                .as_ref()
                                                .and_then(|emoji| tag_emoji(emoji, 14., cx)),
                                        )
                                        .child(tag.name.clone())
                                })),
                        )
                    })
                    .child(
                        div()
                            .px(rpx(12.))
                            .rounded(rpx(8.))
                            .bg(rgb(theme::elevated()))
                            .border_1()
                            .border_color(rgba(theme::border()))
                            .child(
                                Textarea::new(&header.body)
                                    .appearance(false)
                                    .py(rpx(11.))
                                    .text_size(rpx(16.))
                                    .line_height(rpx(22.))
                                    .text_color(rgb(theme::text())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(rpx(8.))
                            .children(header.error.clone().map(|error| {
                                div()
                                    .flex_1()
                                    .text_size(rpx(14.))
                                    .text_color(rgb(theme::RED))
                                    .child(error)
                            }))
                            .child(
                                toolbar_button("new-post-cancel", false)
                                    .rounded(rpx(8.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .on_click(move |_, window, cx| {
                                        cancel
                                            .update(cx, |app, cx| app.cancel_forum_post(window, cx))
                                            .ok();
                                    })
                                    .child("Cancel"),
                            )
                            .child(
                                primary("new-post-submit", true)
                                    .when(header.posting, |this| this.opacity(0.6))
                                    .on_click(move |_, window, cx| {
                                        submit
                                            .update(cx, |app, cx| app.submit_forum_post(window, cx))
                                            .ok();
                                    })
                                    .child("Post"),
                            ),
                    ),
            )
        })
}

fn note(text: &'static str) -> Div {
    div()
        .flex()
        .flex_none()
        .justify_center()
        .py(rpx(16.))
        .text_size(rpx(14.))
        .text_color(rgb(theme::muted()))
        .child(text)
}

fn toolbar(
    app: &DiscordApp,
    forum_id: u64,
    tags: &[ForumTag],
    filter: &[u64],
    cx: &App,
) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .flex_wrap()
        .items_center()
        .gap(rpx(8.))
        .child(sort_view(app, forum_id))
        .when(!tags.is_empty(), |this| {
            this.child(
                div()
                    .flex_none()
                    .w(rpx(2.))
                    .h(rpx(24.))
                    .mx(rpx(8.))
                    .rounded(rpx(4.))
                    .bg(rgba(theme::outline())),
            )
            .children(tags.iter().map(|tag| {
                let entity = app.entity.clone();
                let id = tag.id;
                let selected = filter.contains(&id);
                toolbar_button(("forum-filter", id), selected)
                    .on_click(move |_, _, cx| {
                        entity
                            .update(cx, |app, cx| app.toggle_forum_tag(id, cx))
                            .ok();
                    })
                    .children(
                        tag.emoji
                            .as_ref()
                            .and_then(|emoji| tag_emoji(emoji, 14., cx)),
                    )
                    .child(tag.name.clone())
            }))
        })
        .when(!filter.is_empty(), |this| {
            let entity = app.entity.clone();
            this.child(
                div()
                    .id("forum-filter-clear")
                    .px(rpx(8.))
                    .text_size(rpx(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::muted()))
                    .cursor_pointer()
                    .hover(|style| style.text_color(rgb(theme::strong())).underline())
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |app, cx| app.clear_forum_tags(cx)).ok();
                    })
                    .child("Clear"),
            )
        })
}

fn toolbar_button(id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(6.))
        .h(rpx(32.))
        .px(rpx(12.))
        .rounded(rpx(20.))
        .border_1()
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::strong()))
        .cursor_pointer()
        .map(|this| {
            if selected {
                this.bg(rgba(theme::reaction_selected_bg()))
                    .border_color(rgb(theme::BLURPLE))
            } else {
                this.bg(rgba(theme::control_bg()))
                    .border_color(rgba(theme::outline()))
                    .hover(|style| style.bg(rgba(theme::hover())))
            }
        })
}

fn sort_view(app: &DiscordApp, forum_id: u64) -> impl IntoElement {
    let toggle = app.entity.clone();
    let close = app.entity.clone();
    let sort = app.forum_sort(forum_id);
    let gallery = app.forum_gallery(forum_id);
    let match_all = app.forum_match_all(forum_id);
    let item = |id: &'static str,
                text: &'static str,
                checked: Option<bool>,
                pick: fn(&mut DiscordApp, &mut Context<DiscordApp>)| {
        let entity = app.entity.clone();
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_between()
            .gap(rpx(8.))
            .h(rpx(36.))
            .px(rpx(8.))
            .rounded(rpx(4.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(theme::hover())))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                entity
                    .update(cx, |app, cx| {
                        app.close_forum_menu(cx);
                        pick(app, cx);
                    })
                    .ok();
            })
            .child(text)
            .children(checked.map(radio))
    };
    let heading = |text: &'static str| {
        div()
            .flex()
            .items_center()
            .h(rpx(36.))
            .px(rpx(8.))
            .line_height(rpx(20.))
            .text_color(rgb(theme::faint()))
            .child(text)
    };
    let separator = || div().h(rpx(1.)).my(rpx(8.)).bg(rgba(theme::outline()));
    let menu = app.forum_menu_open().then(|| {
        div().absolute().top(rpx(36.)).left_0().size_0().child(
            deferred(
                anchored().anchor(Anchor::TopLeft).child(
                    div()
                        .id("forum-sort-menu")
                        .flex()
                        .flex_col()
                        .w(rpx(MENU_WIDTH))
                        .p(rpx(8.))
                        .rounded(rpx(8.))
                        .bg(rgb(theme::elevated()))
                        .border_1()
                        .border_color(rgba(theme::outline()))
                        .shadow(vec![BoxShadow {
                            color: rgb(theme::shadow()).opacity(0.24).into(),
                            offset: point(px(0.), px(12.)),
                            blur_radius: px(24.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                        .occlude()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .on_mouse_down_out(move |_, _, cx| {
                            close.update(cx, |app, cx| app.close_forum_menu(cx)).ok();
                        })
                        .child(heading("Sort by"))
                        .child(item(
                            "forum-sort-activity",
                            "Recently Active",
                            Some(sort == ForumSort::Activity),
                            |app, cx| app.set_forum_sort(ForumSort::Activity, cx),
                        ))
                        .child(item(
                            "forum-sort-created",
                            "Date Posted",
                            Some(sort == ForumSort::Created),
                            |app, cx| app.set_forum_sort(ForumSort::Created, cx),
                        ))
                        .child(separator())
                        .child(heading("View as"))
                        .child(item(
                            "forum-view-list",
                            "List",
                            Some(!gallery),
                            |app, cx| app.set_forum_gallery(false, cx),
                        ))
                        .child(item(
                            "forum-view-gallery",
                            "Gallery",
                            Some(gallery),
                            |app, cx| app.set_forum_gallery(true, cx),
                        ))
                        .child(separator())
                        .child(heading("Tag Matching"))
                        .child(item(
                            "forum-match-some",
                            "Match Some",
                            Some(!match_all),
                            |app, cx| app.set_forum_match_all(false, cx),
                        ))
                        .child(item(
                            "forum-match-all",
                            "Match All",
                            Some(match_all),
                            |app, cx| app.set_forum_match_all(true, cx),
                        ))
                        .child(separator())
                        .child(item(
                            "forum-view-reset",
                            "Reset to default",
                            None,
                            |app, cx| app.reset_forum_view(cx),
                        )),
                ),
            )
            .with_priority(1),
        )
    });
    div().relative().flex_none().children(menu).child(
        toolbar_button("forum-sort-view", false)
            .rounded(rpx(8.))
            .gap(rpx(4.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::muted()))
            .on_click(move |_, _, cx| {
                toggle.update(cx, |app, cx| app.toggle_forum_menu(cx)).ok();
            })
            .child(theme::icon("arrow-up-down", 16., theme::muted()).mr(rpx(4.)))
            .child("Sort & View")
            .child(theme::icon("chevron-down", 16., theme::muted())),
    )
}

fn radio(checked: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(20.))
        .rounded_full()
        .map(|this| {
            if checked {
                this.bg(rgb(theme::BLURPLE))
                    .child(div().size(rpx(8.)).rounded_full().bg(white()))
            } else {
                this.border_2().border_color(rgb(theme::muted()))
            }
        })
}

/// Lays cards into rows; the column count comes from the width measured on the previous frame.
fn gallery_grid(app: &DiscordApp, cards: Vec<AnyElement>) -> impl IntoElement {
    let columns = app.forum_columns();
    let measured = columns.clone();
    let entity = app.entity.clone();
    let count = columns.get().max(1);
    let mut rows = Vec::new();
    let mut cards = cards.into_iter().peekable();
    while cards.peek().is_some() {
        let row: Vec<AnyElement> = cards.by_ref().take(count).collect();
        let fillers = count - row.len();
        rows.push(
            div()
                .flex()
                .gap(rpx(GAP))
                .children(
                    row.into_iter()
                        .map(|card| div().flex().flex_1().min_w_0().child(card)),
                )
                .children((0..fillers).map(|_| div().flex_1().min_w_0())),
        );
    }
    div()
        .relative()
        .flex()
        .flex_none()
        .flex_col()
        .gap(rpx(GAP))
        .children(rows)
        .child(
            canvas(
                move |bounds, window, _| {
                    let rem = window.rem_size();
                    let min = rpx(CARD_MIN_WIDTH).to_pixels(rem);
                    let gap = rpx(GAP).to_pixels(rem);
                    let fit = ((bounds.size.width + gap) / (min + gap)).floor().max(1.) as usize;
                    if fit != measured.get() {
                        measured.set(fit);
                        window.on_next_frame(move |_, cx| {
                            entity.update(cx, |_, cx| cx.notify()).ok();
                        });
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
}

fn card_shell(app: &DiscordApp, post: &ForumPost) -> Stateful<Div> {
    let id = post.id;
    let entity = app.entity.clone();
    let open = app.thread_pane_shows(id);
    div()
        .id(("forum-post", id))
        .flex()
        .flex_col()
        .w_full()
        .rounded(rpx(12.))
        .bg(rgb(if open {
            theme::surface_highest()
        } else {
            theme::bg()
        }))
        .border_1()
        .border_color(rgba(theme::border_soft()))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(theme::surface_high())))
        .without_text_selection()
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| app.open_thread(id, window, cx))
                .ok();
        })
}

fn post_author(post: &ForumPost) -> u64 {
    post.first_message
        .as_ref()
        .map_or(post.owner_id, |message| message.author_id)
}

fn author(app: &DiscordApp, post: &ForumPost, cx: &App) -> impl IntoElement {
    let user_id = post_author(post);
    let guild_id = app.selected_guild_id;
    let color = app
        .store
        .member_color(guild_id, user_id)
        .unwrap_or(theme::strong());
    div()
        .flex_none()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(color))
        .child(display_name::render(
            ("forum-author", post.id),
            &author_name(app, user_id),
            app.store.member_name_style(guild_id, user_id),
            Surface::FontOnly,
            cx,
        ))
}

fn new_badge() -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(rpx(16.))
        .px(rpx(6.))
        .rounded(rpx(8.))
        .bg(rgb(theme::BLURPLE))
        .text_size(rpx(12.))
        .font_weight(FontWeight::BOLD)
        .text_color(white())
        .child("NEW")
}

fn gallery_card(
    app: &DiscordApp,
    post: &ForumPost,
    tags: &[ForumTag],
    new: bool,
    cx: &App,
) -> impl IntoElement {
    let post_tags = applied_tags(post, tags);
    let media = post.media();
    let tag_row = tag_row(post, &post_tags, cx);
    let preview = if media.is_empty() {
        text_preview(app, post, tag_row, cx).into_any_element()
    } else {
        mosaic(&media, tag_row, cx).into_any_element()
    };
    card_shell(app, post)
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(8.))
                .min_w_0()
                .mx(rpx(12.))
                .mt(rpx(12.))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .child(author(app, post, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::text()))
                        .child(posted_label(post.created_at)),
                )
                .when(new, |this| this.child(new_badge())),
        )
        .child(
            div()
                .mx(rpx(12.))
                .mt(rpx(4.))
                .h(rpx(20.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(label(("forum-post-title", post.id), &post.name)),
        )
        .child(div().mx(rpx(12.)).mt(rpx(10.)).child(preview))
        .child(div().mt_auto().p(rpx(12.)).child(footer(post, None, cx)))
}

fn applied_tags<'a>(post: &ForumPost, tags: &'a [ForumTag]) -> Vec<&'a ForumTag> {
    post.tag_ids
        .iter()
        .filter_map(|tag| tags.iter().find(|candidate| candidate.id == *tag))
        .collect()
}

/// The pin marker and the first few tags, shown over media or at the foot of the text box.
fn tag_row(post: &ForumPost, tags: &[&ForumTag], cx: &App) -> Option<Div> {
    if tags.is_empty() && !post.pinned {
        return None;
    }
    let hidden = tags.len().saturating_sub(VISIBLE_TAGS);
    Some(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rpx(8.))
            .when(post.pinned, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(rpx(24.))
                        .rounded_full()
                        .bg(rgb(theme::BLURPLE))
                        .child(theme::icon("pin", 14., 0xFFFFFF)),
                )
            })
            .children(tags.iter().take(VISIBLE_TAGS).map(|tag| {
                card_tag()
                    .children(
                        tag.emoji
                            .as_ref()
                            .and_then(|emoji| tag_emoji(emoji, 12., cx)),
                    )
                    .child(tag.name.clone())
            }))
            .when(hidden > 0, |this| {
                this.child(card_tag().child(format!("+{hidden}")))
            }),
    )
}

pub(in crate::app) fn post_tags(app: &DiscordApp, post: &ForumPost, cx: &App) -> Option<Div> {
    let tags = app.store.forum_tags(post.forum_id);
    let applied = applied_tags(post, &tags);
    (!applied.is_empty()).then(|| {
        div()
            .flex()
            .flex_wrap()
            .gap(rpx(8.))
            .children(applied.iter().map(|tag| {
                card_tag()
                    .children(
                        tag.emoji
                            .as_ref()
                            .and_then(|emoji| tag_emoji(emoji, 12., cx)),
                    )
                    .child(tag.name.clone())
            }))
    })
}

fn card_tag() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(6.))
        .h(rpx(24.))
        .px(rpx(8.))
        .rounded(rpx(20.))
        .border_1()
        .border_color(rgba(theme::outline()))
        .bg(rgb(theme::surface_high()))
        .shadow(vec![BoxShadow {
            color: rgb(theme::shadow()).opacity(0.2).into(),
            offset: point(px(0.), px(1.)),
            blur_radius: px(0.),
            spread_radius: px(0.),
            inset: false,
        }])
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::strong()))
}

fn text_preview(
    app: &DiscordApp,
    post: &ForumPost,
    tags: Option<Div>,
    cx: &App,
) -> impl IntoElement {
    let body = post.first_message.as_ref().map(|message| {
        let parsed = app.parsed(message.id, &message.content);
        render_body(
            &parsed.blocks,
            message.edited,
            false,
            &body_context(app, message.id),
            cx,
        )
    });
    div()
        .relative()
        .h(rpx(PREVIEW_HEIGHT))
        .overflow_hidden()
        .p(rpx(12.))
        .rounded(rpx(8.))
        .border_1()
        .border_color(rgba(theme::border_soft()))
        .text_size(rpx(16.))
        .line_height(rpx(22.))
        .text_color(rgb(theme::text()))
        .child(body.unwrap_or_else(|| {
            div()
                .text_color(rgb(theme::muted()))
                .child("No message")
                .into_any_element()
        }))
        .children(tags.map(|tags| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .px(rpx(6.))
                .pt(rpx(24.))
                .pb(rpx(6.))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(rgba(theme::bg() << 8), 0.),
                    linear_color_stop(rgb(theme::bg()), 0.5),
                ))
                .child(tags)
        }))
}

/// One image fills the box, two split it, three or more put one large on the left.
fn mosaic(media: &[ImageSpec], tags: Option<Div>, cx: &App) -> impl IntoElement {
    let radius = rpx(8.);
    let tile = |spec: &ImageSpec| {
        let image = images::resolve(spec, theme::image_px(CARD_MIN_WIDTH), cx);
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(rgb(theme::elevated()))
            .children(image.map(|path| img(path).size_full().object_fit(ObjectFit::Cover)))
    };
    let body = match media {
        [one] => tile(one).size_full().rounded(radius),
        [left, right] => div()
            .flex()
            .gap(rpx(MOSAIC_GAP))
            .size_full()
            .child(tile(left).rounded_l(radius))
            .child(tile(right).rounded_r(radius)),
        [left, top, bottom, ..] => div()
            .flex()
            .gap(rpx(MOSAIC_GAP))
            .size_full()
            .child(tile(left).rounded_l(radius))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(rpx(MOSAIC_GAP))
                    .child(tile(top).rounded_tr(radius))
                    .child(tile(bottom).rounded_br(radius)),
            ),
        [] => div(),
    };
    div()
        .relative()
        .h(rpx(PREVIEW_HEIGHT))
        .child(body)
        .children(tags.map(|tags| {
            div()
                .absolute()
                .left(rpx(6.))
                .bottom(rpx(6.))
                .right(rpx(6.))
                .child(tags)
        }))
}

fn footer(post: &ForumPost, trailing: Option<String>, cx: &App) -> impl IntoElement {
    let reaction = post.first_message.as_ref().and_then(|message| {
        message
            .reactions
            .iter()
            .max_by_key(|reaction| reaction.count)
    });
    div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .h(rpx(24.))
        .text_size(rpx(14.))
        .line_height(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::muted()))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(4.))
                .child(theme::icon("message-circle", 16., theme::faint()))
                .child(post.message_count.to_string()),
        )
        .children(trailing)
        .when(post.locked || post.archived, |this| {
            this.child(if post.locked { "Locked" } else { "Archived" })
        })
        .child(div().flex_1())
        .children(reaction.map(|reaction| reaction_pill(reaction, cx)))
}

fn reaction_pill(reaction: &Reaction, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(6.))
        .h(rpx(28.))
        .px(rpx(6.))
        .rounded(rpx(8.))
        .border_1()
        .map(|this| {
            if reaction.me {
                this.bg(rgba(theme::reaction_selected_bg()))
                    .border_color(rgb(theme::BLURPLE))
            } else {
                this.bg(rgba(theme::control_bg()))
                    .border_color(transparent_black())
            }
        })
        .child(emoji_element(&reaction.emoji, 20., cx))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(if reaction.me {
                    theme::mention_text()
                } else {
                    theme::muted()
                }))
                .child(reaction.count.to_string()),
        )
}

fn tag_emoji(emoji: &StatusEmoji, size: f32, cx: &App) -> Option<AnyElement> {
    let source: ImageSource = match emoji {
        StatusEmoji::Unicode(sequence) => emoji::image(sequence, theme::image_px(size))?.into(),
        StatusEmoji::Custom { id, animated, .. } => images::resolve(
            &ImageSpec::Emoji {
                id: *id,
                animated: *animated,
            },
            theme::image_px(size),
            cx,
        )?
        .into(),
    };
    Some(img(source).size(rpx(size)).flex_none().into_any_element())
}

fn list_card(
    app: &DiscordApp,
    post: &ForumPost,
    tags: &[ForumTag],
    new: bool,
    cx: &App,
) -> impl IntoElement {
    let id = post.id;
    let post_tags = applied_tags(post, tags);
    let preview = post.first_message.as_ref().and_then(|message| {
        let parsed = app.parsed(message.id, &message.content);
        let mut counter = 0;
        let pieces = preview_pieces(
            &parsed.blocks,
            &body_context(app, message.id),
            &mut counter,
            cx,
        );
        (!pieces.is_empty()).then(|| {
            rich_text(("forum-post-preview", id), pieces)
                .single_line()
                .line_height(rpx(18.))
                .emoji_size(rpx(16.))
                .into_any_element()
        })
    });
    let thumbnail = post
        .media()
        .first()
        .and_then(|spec| images::resolve(spec, theme::image_px(THUMBNAIL), cx));
    card_shell(app, post)
        .flex_row()
        .gap(rpx(16.))
        .p(rpx(16.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(6.))
                .flex_1()
                .min_w_0()
                .children(tag_row(post, &post_tags, cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .font_weight(if post.unread {
                                    FontWeight::BOLD
                                } else {
                                    FontWeight::SEMIBOLD
                                })
                                .text_color(rgb(if post.unread {
                                    theme::strong()
                                } else {
                                    theme::text()
                                }))
                                .child(label(("forum-post-title", id), &post.name)),
                        )
                        .when(new, |this| this.child(new_badge())),
                )
                .child(
                    div()
                        .flex()
                        .gap(rpx(4.))
                        .min_w_0()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .child(author(app, post, cx))
                        .child(div().flex_none().text_color(rgb(theme::muted())).child(":"))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(theme::muted()))
                                .child(preview.unwrap_or_else(|| "No message".into_any_element())),
                        ),
                )
                .child(footer(post, Some(relative_time(post.last_activity)), cx)),
        )
        .children(thumbnail.map(|path| {
            img(path)
                .flex_none()
                .size(rpx(THUMBNAIL))
                .rounded(rpx(8.))
                .object_fit(ObjectFit::Cover)
        }))
}

fn minutes_since(timestamp: Timestamp) -> Option<i64> {
    let at = NaiveDate::from_ymd_opt(
        i32::from(timestamp.year),
        u32::from(timestamp.month),
        u32::from(timestamp.day),
    )
    .and_then(|date| date.and_hms_opt(u32::from(timestamp.hour), u32::from(timestamp.minute), 0))
    .and_then(|at| Local.from_local_datetime(&at).single())?;
    Some((Local::now() - at).num_minutes().max(0))
}

pub(super) fn relative_time(timestamp: Timestamp) -> String {
    let Some(minutes) = minutes_since(timestamp) else {
        return timestamp.date();
    };
    match minutes {
        0 => "just now".to_owned(),
        1..60 => format!("{minutes}m ago"),
        60..1440 => format!("{}h ago", minutes / 60),
        1440..43200 => format!("{}d ago", minutes / 1440),
        _ => timestamp.date(),
    }
}

/// Gallery cards say "Posted 5d ago" for the last month, ">30d ago" within the year, then the date.
fn posted_label(created: Timestamp) -> String {
    match minutes_since(created) {
        Some(..43200) => format!("Posted {}", relative_time(created)),
        Some(43200..525600) => ">30d ago".to_owned(),
        _ => created.date(),
    }
}
