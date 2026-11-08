use super::*;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

const SUGGESTIONS: usize = 10;
const MEMBER_QUERY_DELAY: Duration = Duration::from_millis(300);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MentionChoice {
    User {
        id: u64,
        name: String,
        username: String,
    },
    Everyone,
    Here,
}

#[derive(Default)]
pub(crate) struct Suggest {
    index: usize,
    dismissed: Option<String>,
    query: Option<String>,
    // The gateway query API exposes no completion ID; send each prefix once per draft.
    requested: HashSet<String>,
    debounce: Option<Task<()>>,
}

impl DiscordApp {
    fn mention_query(&self, cx: &App) -> Option<(Range<usize>, String)> {
        let input = self.suggest_input().read(cx);
        if !input.selected_range().is_empty() {
            return None;
        }
        mention_query_at(&input.value(), input.cursor())
    }

    pub(crate) fn mention_suggestions(&self, cx: &App) -> Option<(String, Vec<MentionChoice>)> {
        let (_, query) = self.mention_query(cx)?;
        if self.mention_suggest.dismissed.as_deref() == Some(query.as_str()) {
            return None;
        }
        let query_lower = query.to_lowercase();
        let guild_id = self.request_guild();
        let mut special = Vec::new();
        if guild_id.is_some() && self.store.can_mention_everyone(self.selected_channel_id) {
            if "everyone".contains(&query_lower) {
                special.push(MentionChoice::Everyone);
            }
            if "here".contains(&query_lower) {
                special.push(MentionChoice::Here);
            }
        }
        let mut choices: Vec<_> = self
            .store
            .search_users(
                guild_id,
                self.selected_channel_id,
                &query_lower,
                SUGGESTIONS - special.len(),
            )
            .into_iter()
            .filter_map(|id| {
                let user = self.store.user(id)?;
                let name = guild_id
                    .and_then(|guild| self.store.member_name(guild, id))
                    .unwrap_or(&user.name);
                Some(MentionChoice::User {
                    id,
                    name: name.to_owned(),
                    username: user.username.clone(),
                })
            })
            .collect();
        choices.extend(special);
        (!choices.is_empty()).then_some((query, choices))
    }

    pub(super) fn reveal_suggestion(&self, index: usize) {
        // The popup's title is child 0.
        self.suggest_scroll.scroll_to_item(index + 1);
    }

    pub(crate) fn mention_suggest_index(&self) -> usize {
        self.mention_suggest.index
    }

    pub(super) fn reset_mention_suggest(&mut self, cx: &mut Context<Self>) {
        if self.editing.is_none() && self.composer.read(cx).value().is_empty() {
            self.mention_names.clear();
            self.mention_suggest.requested.clear();
        }
        let query = self.mention_query(cx).map(|(_, query)| query);
        if query == self.mention_suggest.query {
            return;
        }
        self.mention_suggest.query = query.clone();
        self.mention_suggest.index = 0;
        self.mention_suggest.dismissed = None;
        self.mention_suggest.debounce = None;
        cx.notify();
        let Some(query) = query.filter(|query| !query.is_empty()) else {
            return;
        };
        let Some(guild_id) = self.request_guild() else {
            return;
        };
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        if self.mention_suggest.requested.contains(&query) {
            return;
        }
        let channel_id = self.selected_channel_id;
        self.mention_suggest.debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(MEMBER_QUERY_DELAY).await;
            this.update(cx, |app, cx| {
                if app.selected_channel_id == channel_id
                    && app.request_guild() == Some(guild_id)
                    && app
                        .mention_query(cx)
                        .is_some_and(|(_, current)| current == query)
                    && app.mention_suggest.dismissed.as_deref() != Some(query.as_str())
                    && app.mention_suggest.requested.insert(query.clone())
                {
                    session.query_members(guild_id, &query, SUGGESTIONS as u32);
                }
            })
            .ok();
        }));
    }

    pub(super) fn reset_mention_draft(&mut self) {
        self.mention_names.clear();
        self.mention_suggest = Suggest::default();
    }

    pub(super) fn move_mention_suggestion(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some((_, choices)) = self.mention_suggestions(cx) else {
            return false;
        };
        self.mention_suggest.index = (self.mention_suggest.index.min(choices.len() - 1) as isize
            + delta)
            .rem_euclid(choices.len() as isize) as usize;
        self.reveal_suggestion(self.mention_suggest.index);
        cx.notify();
        true
    }

    pub(crate) fn accept_mention_suggestion(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((_, choices)) = self.mention_suggestions(cx) else {
            return false;
        };
        let Some((range, _)) = self.mention_query(cx) else {
            return false;
        };
        let index = index
            .unwrap_or(self.mention_suggest.index)
            .min(choices.len() - 1);
        let names = match &mut self.editing {
            Some(editing) => &mut editing.mentions,
            None => &mut self.mention_names,
        };
        let text = mention_insertion(&choices[index], names);
        self.suggest_input().clone().update(cx, |input, cx| {
            input.set_selected_range(range, cx);
            input.replace(text, window, cx);
            input.focus(window, cx);
        });
        self.mention_suggest.index = 0;
        self.mention_suggest.dismissed = None;
        self.mention_suggest.debounce = None;
        cx.notify();
        true
    }

    pub(crate) fn mention_user(
        &mut self,
        user_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(user) = self.store.user(user_id) else {
            return;
        };
        let choice = MentionChoice::User {
            id: user_id,
            name: self
                .store
                .member_name(self.selected_guild_id, user_id)
                .unwrap_or(&user.name)
                .to_owned(),
            username: user.username.clone(),
        };
        let text = mention_insertion(&choice, &mut self.mention_names);
        let joined = {
            let input = self.composer.read(cx);
            let value = input.text().to_string();
            let cursor = input.cursor().min(value.len());
            value[..cursor]
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_whitespace())
        };
        let text = if joined { format!(" {text}") } else { text };
        self.composer.update(cx, |input, cx| {
            input.insert(text, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn dismiss_mention_suggestions(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((query, _)) = self.mention_suggestions(cx) else {
            return false;
        };
        self.mention_suggest.dismissed = Some(query);
        self.mention_suggest.debounce = None;
        cx.notify();
        true
    }

    pub(crate) fn hover_mention_suggestion(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some((_, choices)) = self.mention_suggestions(cx) else {
            return;
        };
        let index = index.min(choices.len() - 1);
        if self.mention_suggest.index != index {
            self.mention_suggest.index = index;
            cx.notify();
        }
    }
}

fn mention_query_at(text: &str, cursor: usize) -> Option<(Range<usize>, String)> {
    let prefix = text.get(..cursor)?;
    let name_start = prefix
        .char_indices()
        .rev()
        .find(|(_, c)| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.')))
        .map_or(0, |(offset, c)| offset + c.len_utf8());
    let at = name_start.checked_sub(1)?;
    if prefix.get(at..name_start)? != "@"
        || prefix[..at]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace())
    {
        return None;
    }
    Some((at..cursor, prefix[name_start..].to_owned()))
}

fn mention_insertion(choice: &MentionChoice, names: &mut HashMap<String, u64>) -> String {
    match choice {
        MentionChoice::Everyone => "@everyone ".to_owned(),
        MentionChoice::Here => "@here ".to_owned(),
        MentionChoice::User { id, name, username } => {
            // Never retarget an earlier pick when two members share a display name.
            let label = [name, username].into_iter().find(|label| {
                !label.is_empty()
                    && !matches!(label.as_str(), "everyone" | "here")
                    && names.get(label.as_str()).is_none_or(|found| found == id)
            });
            match label {
                Some(label) => {
                    names.insert(label.clone(), *id);
                    format!("@{label} ")
                }
                None => format!("<@{id}> "),
            }
        }
    }
}

pub(super) fn encode_mentions(content: &str, names: &HashMap<String, u64>) -> String {
    if names.is_empty() {
        return content.to_owned();
    }
    let mut names: Vec<_> = names.iter().filter(|(name, _)| !name.is_empty()).collect();
    names.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(b.0)));
    let mut output = String::with_capacity(content.len());
    let mut offset = 0;
    while offset < content.len() {
        let tail = &content[offset..];
        if let Some((_, len)) = wire_mention(tail) {
            output.push_str(&tail[..len]);
            offset += len;
            continue;
        }
        if let Some(tail) = tail.strip_prefix('@')
            && let Some((name, id)) = names.iter().find(|(name, _)| {
                tail.starts_with(name.as_str())
                    && mention_name_boundary(content, offset + 1 + name.len())
            })
        {
            output.push_str(&format!("<@{id}>"));
            offset += 1 + name.len();
            continue;
        }
        let c = tail.chars().next().unwrap();
        output.push(c);
        offset += c.len_utf8();
    }
    output
}

pub(super) fn decode_mentions(
    content: &str,
    mut resolve: impl FnMut(u64) -> Option<String>,
) -> (String, HashMap<String, u64>) {
    let mut output = String::with_capacity(content.len());
    let mut names = HashMap::new();
    let mut offset = 0;
    while offset < content.len() {
        let tail = &content[offset..];
        if let Some((id, len)) = wire_mention(tail) {
            if let Some(name) = resolve(id).filter(|name| {
                !name.is_empty()
                    && !matches!(name.as_str(), "everyone" | "here")
                    && !content.contains(&format!("@{name}"))
                    && names.get(name).is_none_or(|found| *found == id)
                    && mention_name_boundary(&format!("{name}{}", &tail[len..]), name.len())
            }) {
                output.push('@');
                output.push_str(&name);
                names.insert(name, id);
            } else {
                output.push_str(&tail[..len]);
            }
            offset += len;
            continue;
        }
        let c = tail.chars().next().unwrap();
        output.push(c);
        offset += c.len_utf8();
    }
    (output, names)
}

fn mention_name_boundary(content: &str, end: usize) -> bool {
    let Some(after) = content.get(end..) else {
        return false;
    };
    // Combining marks belong to the name, not to the punctuation following it.
    (end == content.len()
        || content
            .grapheme_indices(true)
            .any(|(offset, _)| offset == end))
        && after.graphemes(true).next().is_none_or(|next| {
            next.chars().all(char::is_whitespace) || next.unicode_words().next().is_none()
        })
}

fn wire_mention(text: &str) -> Option<(u64, usize)> {
    let tail = text.strip_prefix("<@")?;
    let tail = tail.strip_prefix('!').unwrap_or(tail);
    let end = tail.find('>')?;
    if tail[..end].is_empty() || !tail[..end].bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let id = tail[..end].parse().ok()?;
    Some((id, text.len() - tail.len() + end + 1))
}

pub(super) fn suggestions(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let (_, choices) = app.mention_suggestions(cx)?;
    let selected = app.mention_suggest_index().min(choices.len() - 1);
    let guild_id = app.request_guild();
    Some(suggestion_popup(
        app,
        "Members".into(),
        choices.iter().enumerate().map(|(index, choice)| {
            let hover = app.entity.clone();
            let click = app.entity.clone();
            let (icon, label, detail) = match choice {
                MentionChoice::User { id, name, username } => (
                    match guild_id {
                        Some(guild_id) => {
                            theme::member_avatar(&app.store, guild_id, *id, 24., None, false, cx)
                                .into_any_element()
                        }
                        None => theme::user_avatar(app.store.user(*id), *id, 24., None, false, cx)
                            .into_any_element(),
                    },
                    name.clone(),
                    crate::settings::shown_username(username),
                ),
                MentionChoice::Everyone => (
                    theme::icon("at-sign", 20., theme::muted()).into_any_element(),
                    "@everyone".to_owned(),
                    "Notify everyone who can see this channel.".to_owned(),
                ),
                MentionChoice::Here => (
                    theme::icon("at-sign", 20., theme::muted()).into_any_element(),
                    "@here".to_owned(),
                    "Notify everyone online who can see this channel.".to_owned(),
                ),
            };
            div()
                .id(("mention-suggestion", index))
                .flex()
                .items_center()
                .gap(rpx(10.))
                .mx(rpx(8.))
                .px(rpx(8.))
                .h(rpx(36.))
                .rounded(rpx(4.))
                .cursor_pointer()
                .when(index == selected, |this| this.bg(rgba(theme::selected())))
                .on_hover(move |hovered, _, cx| {
                    if *hovered {
                        hover
                            .update(cx, |app, cx| app.hover_mention_suggestion(index, cx))
                            .ok();
                    }
                })
                .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                .on_click(move |_, window, cx| {
                    click
                        .update(cx, |app, cx| {
                            app.accept_mention_suggestion(Some(index), window, cx)
                        })
                        .ok();
                })
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(rpx(24.))
                        .child(icon),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(rpx(15.))
                        .text_color(rgb(theme::strong()))
                        .child(label),
                )
                .child(
                    div()
                        .flex_none()
                        .max_w(rpx(280.))
                        .truncate()
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::muted()))
                        .child(detail),
                )
        }),
    ))
}

const EDITOR_SUGGESTION_WIDTH: f32 = 430.;
const SUGGESTION_MAX_HEIGHT: f32 = 200.;

/// Spans the composer like Discord's. In the narrower message editor it keeps a fixed width and
/// is deferred and anchored, so a message being edited near the top of the chat doesn't clip it.
pub(super) fn suggestion_popup(
    app: &DiscordApp,
    title: SharedString,
    rows: impl IntoIterator<Item = Stateful<Div>>,
) -> AnyElement {
    let panel = suggestion_panel(title, None)
        .id("suggestion-popup")
        .max_h(rpx(SUGGESTION_MAX_HEIGHT))
        .overflow_y_scroll()
        .track_scroll(&app.suggest_scroll)
        .children(rows.into_iter().map(|row| row.flex_none()));
    place_suggestion_popup(app, panel)
}

/// The popup's chrome and title; callers add the rows and decide how they scroll. `strong` is
/// the bold part after the title, like the `:query` in "Emoji matching :query".
pub(super) fn suggestion_panel(title: SharedString, strong: Option<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        // Discord's `.scroller` pads only the bottom; the title row brings its own top space.
        .pb(rpx(8.))
        .rounded(rpx(8.))
        .border_1()
        .border_color(rgba(theme::border()))
        .bg(rgb(theme::surface_high()))
        .shadow_lg()
        .occlude()
        .child(
            // Discord's `.contentTitle` (4px/0) inside a row's `.base` (8px) and the row's own
            // 8px sides: 12px above and below, 16px in from the edge.
            div()
                .flex()
                .flex_none()
                .min_w_0()
                .py(rpx(12.))
                .px(rpx(16.))
                .text_size(rpx(14.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::text()))
                .child(div().flex_none().child(title))
                .children(strong.map(|strong| {
                    div()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(theme::muted()))
                        .child(strong)
                })),
        )
}

pub(super) fn place_suggestion_popup(
    app: &DiscordApp,
    panel: impl IntoElement + Styled,
) -> AnyElement {
    if app.editing.is_none() {
        return div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(relative(1.))
            .mb(rpx(8.))
            .child(panel)
            .into_any_element();
    }
    div()
        .absolute()
        .top_0()
        .left_0()
        .child(
            deferred(
                anchored()
                    .anchor(Anchor::BottomLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .pb(rpx(8.))
                            .child(panel.w(rpx(EDITOR_SUGGESTION_WIDTH))),
                    ),
            )
            .with_priority(1),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{
        MentionChoice, decode_mentions, encode_mentions, mention_insertion, mention_query_at,
    };
    use std::collections::HashMap;

    #[test]
    fn mention_query_detects_word_starts_and_cursor() {
        assert_eq!(mention_query_at("@", 1), Some((0..1, String::new())));
        assert_eq!(mention_query_at("@ab", 3), Some((0..3, "ab".into())));
        assert_eq!(mention_query_at("a@b", 3), None);
        assert_eq!(mention_query_at("hi\n@ab", 6), Some((3..6, "ab".into())));
        assert_eq!(
            mention_query_at("@alice suffix", 3),
            Some((0..3, "al".into()))
        );
        assert_eq!(mention_query_at("hello @", 7), Some((6..7, String::new())));
        assert_eq!(mention_query_at("@a.b_c", 6), Some((0..6, "a.b_c".into())));
        assert_eq!(mention_query_at("@ab!", 4), None);
        assert_eq!(mention_query_at("@\u{e9}", 2), None);
        assert_eq!(
            mention_query_at("@\u{e9}", 3),
            Some((0..3, "\u{e9}".into()))
        );
    }

    #[test]
    fn mention_encoding_uses_picked_names_and_longest_boundary_match() {
        let names = HashMap::from([
            ("Al".into(), 1),
            ("Alice".into(), 2),
            ("Alice Smith".into(), 3),
        ]);
        assert_eq!(
            encode_mentions("@Al @Alice, @Alice Smith! @bob @Alicex", &names),
            "<@1> <@2>, <@3>! @bob @Alicex"
        );
        assert_eq!(
            encode_mentions("`@Al` <@1> <@!2>", &names),
            "`<@1>` <@1> <@!2>"
        );
        assert_eq!(encode_mentions("@Al\u{301}", &names), "@Al\u{301}");
        assert_eq!(encode_mentions("@Al\u{ff01}", &names), "<@1>\u{ff01}");
        assert_eq!(encode_mentions("@Al", &names), "<@1>");
        assert_eq!(encode_mentions("@bob", &HashMap::new()), "@bob");
    }

    #[test]
    fn mention_edit_round_trip_preserves_unknown_users() {
        let (text, names) = decode_mentions("<@1> hi <@!2> <@99>", |id| match id {
            1 => Some("Name".into()),
            2 => Some("Other".into()),
            _ => None,
        });
        assert_eq!(text, "@Name hi @Other <@99>");
        assert_eq!(encode_mentions(&text, &names), "<@1> hi <@2> <@99>");
    }

    #[test]
    fn mention_choices_do_not_retarget_duplicate_names_or_track_mass_mentions() {
        let mut names = HashMap::new();
        let first = MentionChoice::User {
            id: 1,
            name: "Name".into(),
            username: "first".into(),
        };
        let second = MentionChoice::User {
            id: 2,
            name: "Name".into(),
            username: "second".into(),
        };
        let text = format!(
            "{}{}",
            mention_insertion(&first, &mut names),
            mention_insertion(&second, &mut names)
        );
        assert_eq!(text, "@Name @second ");
        assert_eq!(encode_mentions(&text, &names), "<@1> <@2> ");
        assert_eq!(
            mention_insertion(&MentionChoice::Everyone, &mut names),
            "@everyone "
        );
        assert_eq!(
            mention_insertion(&MentionChoice::Here, &mut names),
            "@here "
        );
        assert!(!names.contains_key("everyone"));
        assert!(!names.contains_key("here"));
        let (text, names) = decode_mentions("<@1> <@2>", |_| Some("Same".into()));
        assert_eq!(encode_mentions(&text, &names), "<@1> <@2>");
    }

    #[test]
    fn mention_edit_round_trip_keeps_adjacent_words_and_malformed_tokens_literal() {
        let content = "<@1>suffix <@1>\u{301} <@abc> <@&1> <@18446744073709551616>";
        let (text, names) = decode_mentions(content, |_| Some("Name".into()));
        assert_eq!(text, content);
        assert!(names.is_empty());
        assert_eq!(encode_mentions(&text, &names), content);
    }

    #[test]
    fn mention_edit_round_trip_does_not_encode_literal_names() {
        let content = "<@1> and literal @Name";
        let (text, names) = decode_mentions(content, |_| Some("Name".into()));
        assert_eq!(text, content);
        assert!(names.is_empty());
        assert_eq!(encode_mentions(&text, &names), content);
    }
}
