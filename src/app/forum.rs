use chrono::{Local, NaiveDate, TimeZone};
use gpui::{prelude::*, *};

use crate::{
    app::{
        DiscordApp,
        chat::{author_name, body_context},
        message_body::{label, preview_pieces},
        rich_text::rich_text,
    },
    emoji, images,
    store::{ForumPost, ForumTag, StatusEmoji, Timestamp},
    theme::{self, WithoutTextSelection, rpx},
};

const THUMBNAIL: f32 = 72.;

pub fn render(app: &DiscordApp, forum_id: u64, cx: &App) -> impl IntoElement + use<> {
    let tags = app.store.forum_tags(forum_id);
    let posts = app.store.forum_posts(forum_id);
    let loading = app.forum_loading();
    let more = app.forum_has_more();
    let entity = app.entity.clone();
    div()
        .id(("forum", forum_id))
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .flex_1()
        .min_h_0()
        .w_full()
        .px(rpx(16.))
        .py(rpx(16.))
        .overflow_y_scroll()
        .when(!tags.is_empty(), |this| {
            this.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(rpx(6.))
                    .pb(rpx(4.))
                    .children(tags.iter().map(|tag| tag_chip(tag, cx))),
            )
        })
        .children(
            posts
                .iter()
                .map(|post| post_card(app, post, &tags, cx).into_any_element()),
        )
        .when(posts.is_empty() && !loading, |this| {
            this.child(
                div()
                    .flex()
                    .justify_center()
                    .py(rpx(48.))
                    .text_size(rpx(14.))
                    .text_color(rgb(theme::MUTED))
                    .child("No posts yet"),
            )
        })
        .when(loading, |this| {
            this.child(
                div()
                    .flex()
                    .justify_center()
                    .py(rpx(16.))
                    .text_size(rpx(14.))
                    .text_color(rgb(theme::MUTED))
                    .child("Loading posts…"),
            )
        })
        .when(more && !loading, |this| {
            this.child(
                div()
                    .id("forum-more")
                    .flex()
                    .justify_center()
                    .py(rpx(10.))
                    .rounded(rpx(8.))
                    .text_size(rpx(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::TEXT))
                    .bg(rgb(theme::ELEVATED))
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(theme::HOVER)))
                    .child("Load more posts")
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |app, cx| app.load_more_forum(cx)).ok();
                    }),
            )
        })
}

fn tag_chip(tag: &ForumTag, cx: &App) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(4.))
        .h(rpx(24.))
        .px(rpx(8.))
        .rounded(rpx(12.))
        .bg(rgb(theme::SELECTED))
        .text_size(rpx(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::TEXT))
        .children(tag.emoji.as_ref().and_then(|emoji| tag_emoji(emoji, cx)))
        .child(tag.name.clone())
}

fn tag_emoji(emoji: &StatusEmoji, cx: &App) -> Option<AnyElement> {
    let source: ImageSource = match emoji {
        StatusEmoji::Unicode(sequence) => emoji::image(sequence)?.into(),
        StatusEmoji::Custom { id, animated, .. } => images::resolve(
            &images::ImageSpec::Emoji {
                id: *id,
                animated: *animated,
            },
            theme::image_px(14.),
            cx,
        )?
        .into(),
    };
    Some(img(source).size(rpx(14.)).flex_none().into_any_element())
}

fn post_card(app: &DiscordApp, post: &ForumPost, tags: &[ForumTag], cx: &App) -> impl IntoElement {
    let id = post.id;
    let entity = app.entity.clone();
    let post_tags: Vec<&ForumTag> = post
        .tag_ids
        .iter()
        .filter_map(|tag| tags.iter().find(|candidate| candidate.id == *tag))
        .collect();
    let author = author_name(app, post.owner_id);
    // A forum post's first message shares the thread's id.
    let preview = post.preview.as_deref().and_then(|content| {
        let parsed = app.parsed(id, content);
        let mut counter = 0;
        let pieces = preview_pieces(&parsed.blocks, &body_context(app, id), &mut counter, cx);
        (!pieces.is_empty()).then(|| {
            rich_text(("forum-post-preview", id), pieces)
                .single_line()
                .line_height(rpx(18.))
                .emoji_size(rpx(16.))
                .into_any_element()
        })
    });
    let thumbnail = post
        .thumbnail
        .as_ref()
        .and_then(|spec| images::resolve(spec, theme::image_px(THUMBNAIL), cx));
    let title_color = if post.unread {
        theme::STRONG
    } else {
        theme::TEXT
    };
    div()
        .id(("forum-post", id))
        .flex()
        .gap(rpx(16.))
        .w_full()
        .p(rpx(16.))
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(theme::HOVER)))
        .without_text_selection()
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| app.open_thread(id, window, cx))
                .ok();
        })
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(6.))
                .flex_1()
                .min_w_0()
                .when(!post_tags.is_empty() || post.pinned, |this| {
                    this.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(rpx(4.))
                            .when(post.pinned, |this| {
                                this.child(theme::icon("pin", 14., theme::MUTED))
                            })
                            .children(post_tags.iter().map(|tag| tag_chip(tag, cx))),
                    )
                })
                .child(
                    div()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(if post.unread {
                            FontWeight::BOLD
                        } else {
                            FontWeight::SEMIBOLD
                        })
                        .text_color(rgb(title_color))
                        .child(label(("forum-post-title", id), &post.name)),
                )
                .child(
                    div()
                        .flex()
                        .gap(rpx(4.))
                        .min_w_0()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(theme::TEXT))
                                .child(format!("{author}:")),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(rgb(theme::MUTED))
                                .child(preview.unwrap_or_else(|| "No message".into_any_element())),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(12.))
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::MUTED))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(rpx(4.))
                                .child(theme::icon("messages-square", 14., theme::MUTED))
                                .child(post.message_count.to_string()),
                        )
                        .child(relative_time(post.last_activity))
                        .when(post.locked || post.archived, |this| {
                            this.child(if post.locked { "Locked" } else { "Archived" })
                        }),
                ),
        )
        .children(thumbnail.map(|path| {
            img(path)
                .flex_none()
                .size(rpx(THUMBNAIL))
                .rounded(rpx(8.))
                .object_fit(ObjectFit::Cover)
        }))
}

fn relative_time(timestamp: Timestamp) -> String {
    let at = NaiveDate::from_ymd_opt(
        i32::from(timestamp.year),
        u32::from(timestamp.month),
        u32::from(timestamp.day),
    )
    .and_then(|date| date.and_hms_opt(u32::from(timestamp.hour), u32::from(timestamp.minute), 0))
    .and_then(|at| Local.from_local_datetime(&at).single());
    let Some(at) = at else {
        return timestamp.date();
    };
    let minutes = (Local::now() - at).num_minutes().max(0);
    match minutes {
        0 => "just now".to_owned(),
        1..60 => format!("{minutes}m ago"),
        60..1440 => format!("{}h ago", minutes / 60),
        1440..43200 => format!("{}d ago", minutes / 1440),
        _ => timestamp.date(),
    }
}
