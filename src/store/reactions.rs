use super::*;

impl Store {
    pub(super) fn same_emoji(left: &ReactionEmoji, right: &model::Emoji) -> bool {
        match (left.id, right.id) {
            (Some(left), Some(right)) => left == right,
            (None, None) => Some(left.name.as_str()) == right.name.as_deref(),
            _ => false,
        }
    }

    pub(super) fn add_reaction(
        reactions: &mut Vec<Reaction>,
        emoji: &model::Emoji,
        count: u32,
        me: bool,
    ) {
        if count == 0 || (emoji.id.is_none() && emoji.name.as_deref().is_none_or(str::is_empty)) {
            return;
        }
        if let Some(reaction) = reactions
            .iter_mut()
            .find(|r| Self::same_emoji(&r.emoji, emoji))
        {
            reaction.count = reaction.count.saturating_add(count);
            reaction.me |= me;
        } else {
            reactions.push(Reaction {
                emoji: mapping::map_emoji(emoji),
                count,
                me,
            });
        }
    }

    pub(super) fn change_reactions(
        &mut self,
        channel_id: u64,
        message_id: u64,
        update: impl FnOnce(&mut Vec<Reaction>),
    ) -> Change {
        let Some(channel) = self
            .network_channel_mut(channel_id)
            .filter(|channel| channel.messages_loaded)
        else {
            return Change::None;
        };
        let Ok(index) = channel
            .messages
            .binary_search_by_key(&message_id, |message| message.id)
        else {
            return Change::None;
        };
        update(&mut channel.messages[index].reactions);
        Change::MessageChanged { channel_id, index }
    }

    pub fn reaction_state(
        &self,
        channel_id: u64,
        message_id: u64,
        emoji: &ReactionEmoji,
    ) -> Option<bool> {
        let channel = self.network_channel(channel_id)?;
        let index = channel
            .messages
            .binary_search_by_key(&message_id, |message| message.id)
            .ok()?;
        channel.messages[index]
            .reactions
            .iter()
            .find(|reaction| Self::same_view_emoji(&reaction.emoji, emoji))
            .map(|reaction| reaction.me)
    }

    pub fn set_reaction(
        &mut self,
        channel_id: u64,
        message_id: u64,
        emoji: &ReactionEmoji,
        me: bool,
    ) -> Change {
        let Some(channel) = self
            .network_channel_mut(channel_id)
            .filter(|channel| channel.messages_loaded)
        else {
            return Change::None;
        };
        let Ok(index) = channel
            .messages
            .binary_search_by_key(&message_id, |message| message.id)
        else {
            return Change::None;
        };
        let reactions = &mut channel.messages[index].reactions;
        let existing = reactions
            .iter()
            .position(|reaction| Self::same_view_emoji(&reaction.emoji, emoji));
        if me {
            match existing {
                Some(reaction) => {
                    if reactions[reaction].me {
                        return Change::None;
                    }
                    reactions[reaction].me = true;
                    reactions[reaction].count = reactions[reaction].count.saturating_add(1);
                }
                None => reactions.push(Reaction {
                    emoji: emoji.clone(),
                    count: 1,
                    me: true,
                }),
            }
        } else {
            let Some(reaction) = existing else {
                return Change::None;
            };
            if !reactions[reaction].me {
                return Change::None;
            }
            reactions[reaction].me = false;
            reactions[reaction].count = reactions[reaction].count.saturating_sub(1);
            if reactions[reaction].count == 0 {
                reactions.remove(reaction);
            }
        }
        Change::MessageChanged { channel_id, index }
    }

    fn same_view_emoji(left: &ReactionEmoji, right: &ReactionEmoji) -> bool {
        match (left.id, right.id) {
            (Some(left), Some(right)) => left == right,
            (None, None) => left.name == right.name,
            _ => false,
        }
    }

    pub(super) fn reaction_add(&mut self, event: model::ReactionEvent) -> Change {
        let current_user_id = self.current_user_id;
        let own = event.user_id == Some(current_user_id);
        self.change_reactions(event.channel_id, event.message_id, |reactions| {
            // Our optimistic toggle already added it; the echo must not double count.
            if own
                && reactions
                    .iter()
                    .any(|reaction| reaction.me && Self::same_emoji(&reaction.emoji, &event.emoji))
            {
                return;
            }
            Self::add_reaction(reactions, &event.emoji, 1, own);
        })
    }

    pub(super) fn reaction_remove(&mut self, event: model::ReactionEvent) -> Change {
        let current_user_id = self.current_user_id;
        let own = event.user_id == Some(current_user_id);
        self.change_reactions(event.channel_id, event.message_id, |reactions| {
            if let Some(index) = reactions
                .iter()
                .position(|r| Self::same_emoji(&r.emoji, &event.emoji))
            {
                // Our optimistic toggle already removed it.
                if own && !reactions[index].me {
                    return;
                }
                let reaction = &mut reactions[index];
                reaction.count = reaction.count.saturating_sub(1);
                if own {
                    reaction.me = false;
                }
                if reaction.count == 0 {
                    reactions.remove(index);
                }
            }
        })
    }

    pub(super) fn reaction_add_many(&mut self, event: model::ReactionManyEvent) -> Change {
        let current_user_id = self.current_user_id;
        self.change_reactions(event.channel_id, event.message_id, |reactions| {
            for reaction in event.reactions {
                let own = reaction.users.contains(&current_user_id);
                let mut count = reaction.users.len().try_into().unwrap_or(u32::MAX);
                if own
                    && reactions
                        .iter()
                        .any(|r| r.me && Self::same_emoji(&r.emoji, &reaction.emoji))
                {
                    count = count.saturating_sub(1);
                }
                Self::add_reaction(reactions, &reaction.emoji, count, own);
            }
        })
    }
}
