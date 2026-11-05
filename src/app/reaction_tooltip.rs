//! The tooltip over a reaction naming who reacted, as Discord shows it: the emoji large beside
//! ":name: reacted by A, B, C, and N others". The names come from
//! `GET /channels/{id}/messages/{id}/reactions/{emoji}` when the reaction is first hovered.

use super::*;
use std::{ops::Range, time::Duration};

use crate::{discord::model, store::Reaction};

/// Discord names at most three reactors and counts the rest.
const NAMED_REACTORS: u32 = 3;
/// How long the tooltip outlasts the pointer, so it can cross the gap onto the tooltip.
const LEAVE_GRACE: Duration = Duration::from_millis(150);

type ReactionKey = (u64, String);

struct Reactors {
    /// The reaction's count when these were fetched.
    count: u32,
    /// Matches the fetch that owns this entry, so an older reply can't land in it.
    request: u64,
    /// `None` while loading.
    users: Option<Vec<model::User>>,
    /// Shown until a refetch replaces them.
    stale: bool,
}

#[derive(Default)]
pub(crate) struct ReactionTips {
    hovered: Option<ReactionKey>,
    over_tooltip: bool,
    /// Bumped on every hover change, so a hide timer from an earlier leave does nothing.
    hover_changes: u64,
    requests: u64,
    users: HashMap<ReactionKey, Reactors>,
}

/// Normal reactors first, then burst ones, at most `limit` and each person once.
fn merge_reactors(
    normal: Vec<model::User>,
    burst: Vec<model::User>,
    limit: usize,
) -> Vec<model::User> {
    let mut users: Vec<model::User> = Vec::with_capacity(limit);
    for user in normal.into_iter().chain(burst) {
        if users.len() == limit {
            break;
        }
        if !users.iter().any(|seen| seen.id == user.id) {
            users.push(user);
        }
    }
    users
}

impl DiscordApp {
    pub(crate) fn set_reaction_hover(
        &mut self,
        message_id: u64,
        reaction: &Reaction,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let key = (message_id, reaction_emoji_text(&reaction.emoji));
        self.reaction_tips.hover_changes += 1;
        if hovered {
            // The pointer is on a pill, so not on any tooltip, whatever a tooltip that unmounted
            // without a leave last reported.
            self.reaction_tips.over_tooltip = false;
            self.reaction_tips.hovered = Some(key);
            self.load_reaction_users(message_id, &reaction.emoji, reaction.count, cx);
            cx.notify();
        } else {
            self.hide_reaction_tooltip_later(key, cx);
        }
    }

    fn set_reaction_tooltip_hover(
        &mut self,
        key: ReactionKey,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        self.reaction_tips.over_tooltip = hovered;
        self.reaction_tips.hover_changes += 1;
        if hovered {
            self.reaction_tips.hovered = Some(key);
        } else {
            self.hide_reaction_tooltip_later(key, cx);
        }
    }

    fn hide_reaction_tooltip_later(&mut self, key: ReactionKey, cx: &mut Context<Self>) {
        let changes = self.reaction_tips.hover_changes;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LEAVE_GRACE).await;
            this.update(cx, |app, cx| {
                let tips = &mut app.reaction_tips;
                // The pointer can reach the tooltip before the pill's leave is handled, so the
                // tooltip's own hover is checked too.
                if tips.hover_changes == changes
                    && !tips.over_tooltip
                    && tips.hovered.as_ref() == Some(&key)
                {
                    tips.hovered = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// A message's reactions changed on Discord: forgets who reacted to each of them.
    pub(crate) fn message_reactions_changed(&mut self, message_id: u64, cx: &mut Context<Self>) {
        self.forget_reactors(message_id, None, cx);
    }

    /// Your own reaction landed on Discord: forgets who reacted with it, so the list includes or
    /// drops you.
    pub(crate) fn reaction_changed(
        &mut self,
        message_id: u64,
        emoji: &ReactionEmoji,
        cx: &mut Context<Self>,
    ) {
        self.forget_reactors(message_id, Some(&reaction_emoji_text(emoji)), cx);
    }

    /// Drops the users known for a message's reactions, all or only `emoji`'s. The hovered one
    /// keeps showing what it had until the refetch lands, since unmounting the tooltip under the
    /// pointer would lose its hover state.
    fn forget_reactors(&mut self, message_id: u64, emoji: Option<&str>, cx: &mut Context<Self>) {
        let matches = |(message, text): &ReactionKey| {
            *message == message_id && emoji.is_none_or(|emoji| emoji == text)
        };
        let hovered = self
            .reaction_tips
            .hovered
            .clone()
            .filter(|key| matches(key));
        self.reaction_tips
            .users
            .retain(|key, _| !matches(key) || Some(key) == hovered.as_ref());
        let Some(key) = hovered else {
            return;
        };
        if let Some(reactors) = self.reaction_tips.users.get_mut(&key) {
            reactors.stale = true;
        }
        let reaction = self
            .message_reactions(message_id)
            .iter()
            .find(|reaction| reaction_emoji_text(&reaction.emoji) == key.1)
            .cloned();
        match reaction {
            Some(reaction) => {
                self.load_reaction_users(message_id, &reaction.emoji, reaction.count, cx)
            }
            None => self.dismiss_reaction_tooltip(),
        }
        cx.notify();
    }

    /// The message is gone: forgets its reactions' users, and its tooltip if one is up.
    pub(crate) fn message_reactions_removed(&mut self, message_id: u64) {
        self.reaction_tips
            .users
            .retain(|(message, _), _| *message != message_id);
        if self
            .reaction_tips
            .hovered
            .as_ref()
            .is_some_and(|(message, _)| *message == message_id)
        {
            self.dismiss_reaction_tooltip();
        }
    }

    /// Takes the tooltip down without a pointer leaving it, which gpui wouldn't report.
    pub(crate) fn dismiss_reaction_tooltip(&mut self) {
        let tips = &mut self.reaction_tips;
        tips.hovered = None;
        tips.over_tooltip = false;
        tips.hover_changes += 1;
    }

    /// Fetches who reacted unless the list was already fetched at this `count`.
    fn load_reaction_users(
        &mut self,
        message_id: u64,
        emoji: &ReactionEmoji,
        count: u32,
        cx: &mut Context<Self>,
    ) {
        let text = reaction_emoji_text(emoji);
        let key = (message_id, text.clone());
        if self
            .reaction_tips
            .users
            .get(&key)
            .is_some_and(|reactors| !reactors.stale && reactors.count == count)
        {
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.reaction_tips.requests += 1;
        let request = self.reaction_tips.requests;
        let stale = self
            .reaction_tips
            .users
            .remove(&key)
            .and_then(|reactors| reactors.users);
        self.reaction_tips.users.insert(
            key.clone(),
            Reactors {
                count,
                request,
                users: stale,
                stale: false,
            },
        );
        let (guild_id, channel_id) = (self.request_guild(), self.selected_channel_id);
        let fetch = Tokio::spawn(cx, async move {
            let page = |burst| model::ReactorPage {
                limit: NAMED_REACTORS,
                after: None,
                burst,
            };
            let normal = session
                .reaction_users(guild_id, channel_id, message_id, &text, page(false))
                .await?;
            // A reaction can be all or partly super reactions, which Discord lists apart.
            let burst = if normal.len() < NAMED_REACTORS as usize {
                session
                    .reaction_users(guild_id, channel_id, message_id, &text, page(true))
                    .await?
            } else {
                Vec::new()
            };
            Ok::<_, discord::Error>(merge_reactors(normal, burst, NAMED_REACTORS as usize))
        });
        cx.spawn(async move |this, cx| {
            let result = fetch.await;
            this.update(cx, |app, cx| {
                let Some(reactors) = app
                    .reaction_tips
                    .users
                    .get_mut(&key)
                    .filter(|reactors| reactors.request == request)
                else {
                    return;
                };
                match result {
                    Ok(Ok(users)) => {
                        reactors.users = Some(users);
                        cx.notify();
                        return;
                    }
                    Ok(Err(error)) => log::warn!("Loading reactors failed: {error}"),
                    Err(error) => log::warn!("Reactor task failed: {error}"),
                }
                // A failed fetch keeps what the tooltip showed, marked for another try. Without
                // anything to show, the tooltip goes, through the dismissal gpui wouldn't report.
                let shown = reactors.users.is_some();
                if shown {
                    reactors.stale = true;
                } else {
                    app.reaction_tips.users.remove(&key);
                    if app.reaction_tips.hovered.as_ref() == Some(&key) {
                        app.dismiss_reaction_tooltip();
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn reaction_tooltip(
        &self,
        message_id: u64,
        reaction: &Reaction,
        cx: &App,
    ) -> Option<AnyElement> {
        let key = (message_id, reaction_emoji_text(&reaction.emoji));
        if self.reaction_tips.hovered.as_ref() != Some(&key) {
            return None;
        }
        let users = self.reaction_tips.users.get(&key)?.users.as_ref()?;
        let names: Vec<String> = users.iter().map(|user| self.reactor_name(user)).collect();
        let (text, others) = reacted_by(&emoji_label(&reaction.emoji), &names, reaction.count);
        let (hover, open) = (self.entity.clone(), self.entity.clone());
        let emoji = reaction.emoji.clone();
        let mut label = StyledText::new(text);
        if let Some(range) = others.clone() {
            label = label.with_highlights([(
                range,
                HighlightStyle {
                    color: Some(rgb(theme::link()).into()),
                    ..Default::default()
                },
            )]);
        }
        let label = InteractiveText::new("reaction-tooltip-text", label).on_click(
            others.into_iter().collect(),
            move |_, window, cx| {
                cx.stop_propagation();
                open.update(cx, |app, cx| {
                    app.dismiss_reaction_tooltip();
                    app.open_reactions_dialog(message_id, Some(&emoji), window, cx);
                })
                .ok();
            },
        );
        let card = theme::tooltip_bubble()
            .id("reaction-tooltip")
            .max_w(rpx(320.))
            .flex()
            .items_center()
            .gap(rpx(12.))
            .p(rpx(16.))
            .rounded(rpx(4.))
            // It floats over other messages, whose links and reactions mustn't take its clicks.
            .occlude()
            .on_hover(move |over, _, cx| {
                hover
                    .update(cx, |app, cx| {
                        app.set_reaction_tooltip_hover(key.clone(), *over, cx)
                    })
                    .ok();
            })
            .child(
                div()
                    .flex_none()
                    .size(rpx(48.))
                    .child(embeds::emoji_element(&reaction.emoji, 48., cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .text_color(rgb(theme::text()))
                    .child(label),
            );
        Some(theme::tooltip_card_above(card, cx).into_any_element())
    }

    pub(super) fn reactor_name(&self, user: &model::User) -> String {
        self.request_guild()
            .and_then(|guild_id| self.store.member_name(guild_id, user.id))
            .map(str::to_owned)
            .or_else(|| user.global_name.clone().filter(|name| !name.is_empty()))
            .unwrap_or_else(|| user.username.clone())
    }
}

/// `:name:` for a custom emoji or a Unicode one with a shortcode, else the emoji itself.
fn emoji_label(emoji: &ReactionEmoji) -> String {
    match emoji.id {
        Some(_) => format!(":{}:", emoji.name),
        None => crate::text::emoji::shortcode_for_emoji(&emoji.name)
            .map_or_else(|| emoji.name.clone(), |name| format!(":{name}:")),
    }
}

/// The tooltip's text, and where its "N others" sits; Discord links that to the dialog.
fn reacted_by(emoji: &str, names: &[String], count: u32) -> (String, Option<Range<usize>>) {
    let others = match (count as usize).saturating_sub(names.len()) {
        0 => None,
        1 => Some("1 other".to_owned()),
        others => Some(format!("{others} others")),
    };
    let mut parts: Vec<&str> = names.iter().map(String::as_str).collect();
    parts.extend(others.as_deref());
    let list = match parts.as_slice() {
        [] => return (emoji.to_owned(), None),
        [one] => (*one).to_owned(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    };
    let text = format!("{emoji} reacted by {list}");
    let link = others.map(|others| text.len() - others.len()..text.len());
    (text, link)
}

#[cfg(test)]
mod tests {
    use super::reacted_by;

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn reaction_tooltips_name_three_reactors_and_count_the_rest() {
        assert_eq!(
            reacted_by(":Donk:", &names(&["Leorad"]), 1).0,
            ":Donk: reacted by Leorad"
        );
        assert_eq!(
            reacted_by(":Donk:", &names(&["A", "B"]), 2).0,
            ":Donk: reacted by A and B"
        );
        assert_eq!(
            reacted_by(":Donk:", &names(&["A", "B", "C"]), 3).0,
            ":Donk: reacted by A, B, and C"
        );
        assert_eq!(
            reacted_by(":Donk:", &names(&["A", "B", "C"]), 4).0,
            ":Donk: reacted by A, B, C, and 1 other"
        );
        let (text, link) = reacted_by(":KUDOGGER:", &names(&["A", "B", "C"]), 5);
        assert_eq!(text, ":KUDOGGER: reacted by A, B, C, and 2 others");
        assert_eq!(&text[link.unwrap()], "2 others");
    }
}
