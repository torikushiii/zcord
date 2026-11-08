use super::*;

#[derive(Clone, Default)]
pub struct DmCall {
    pub channel_id: u64,
    pub message_id: Option<u64>,
    pub region: Option<String>,
    pub ringing: Vec<u64>,
    pub voice_states: Vec<model::VoiceState>,
}

impl Store {
    pub fn dm_call(&self, channel_id: u64) -> Option<&DmCall> {
        self.dm_calls.get(&channel_id)
    }

    pub fn incoming_call(&self) -> Option<u64> {
        self.dm_calls
            .values()
            .filter(|call| call.ringing.contains(&self.current_user_id))
            .map(|call| call.channel_id)
            .min()
    }

    pub fn stop_ringing_locally(&mut self, channel_id: u64) {
        if let Some(call) = self.dm_calls.get_mut(&channel_id) {
            call.ringing.retain(|id| *id != self.current_user_id);
        }
    }

    pub(super) fn call_upsert(&mut self, update: model::PrivateCall, create: bool) -> Change {
        if self.private_channel(update.channel_id).is_none() {
            return Change::None;
        }
        if create {
            let voice_states = self
                .dm_calls
                .remove(&update.channel_id)
                .filter(|call| call.message_id.is_none())
                .map(|call| call.voice_states)
                .unwrap_or_default();
            self.dm_calls.insert(
                update.channel_id,
                DmCall {
                    channel_id: update.channel_id,
                    voice_states,
                    ..Default::default()
                },
            );
        }
        if let Some(states) = &update.voice_states {
            for user in states
                .iter()
                .filter_map(|state| state.member.as_ref()?.user.as_ref())
            {
                self.register_author(user);
            }
        }
        let call = self
            .dm_calls
            .entry(update.channel_id)
            .or_insert_with(|| DmCall {
                channel_id: update.channel_id,
                ..Default::default()
            });
        if let Some(message_id) = update.message_id {
            call.message_id = Some(message_id);
        }
        if let Some(region) = update.region {
            call.region = region;
        }
        if let Some(ringing) = update.ringing {
            call.ringing = ringing;
        }
        if let Some(states) = update.voice_states {
            call.voice_states.clear();
            for mut state in states {
                if state.guild_id.is_some()
                    || state.channel_id.is_some_and(|id| id != call.channel_id)
                {
                    continue;
                }
                state.guild_id = None;
                state.channel_id = Some(call.channel_id);
                call.voice_states
                    .retain(|existing| existing.user_id != state.user_id);
                call.voice_states.push(state);
            }
        }
        Change::Structure
    }

    pub(super) fn dm_voice_state_update(&mut self, state: model::VoiceState) -> Change {
        if let Some(user) = state
            .member
            .as_ref()
            .and_then(|member| member.user.as_ref())
        {
            self.register_author(user);
        }
        for call in self.dm_calls.values_mut() {
            call.voice_states
                .retain(|existing| existing.user_id != state.user_id);
        }
        if let Some(channel_id) = state.channel_id
            && self.private_channel(channel_id).is_some()
        {
            let call = self.dm_calls.entry(channel_id).or_insert_with(|| DmCall {
                channel_id,
                ..Default::default()
            });
            call.voice_states.push(state);
        }
        Change::Structure
    }

    pub(super) fn channel_recipient(
        &mut self,
        update: model::ChannelRecipient,
        add: bool,
    ) -> Change {
        self.register_author(&update.user);
        if !add && update.user.id == self.current_user_id {
            return self.remove_private_locally(update.channel_id);
        }
        let Some(SourceChannel::Private(index)) =
            self.source_channels.get(&update.channel_id).copied()
        else {
            return Change::None;
        };
        if let Some(channel) = self
            .source
            .as_mut()
            .and_then(|source| source.private_channels.get_mut(index))
        {
            if add {
                if !channel.recipient_ids.contains(&update.user.id) {
                    channel.recipient_ids.push(update.user.id);
                }
                channel.recipients.retain(|user| user.id != update.user.id);
                channel.recipients.push(update.user);
            } else {
                channel.recipient_ids.retain(|id| *id != update.user.id);
                channel.recipients.retain(|user| user.id != update.user.id);
                if let Some(call) = self.dm_calls.get_mut(&update.channel_id) {
                    call.ringing.retain(|id| *id != update.user.id);
                    call.voice_states
                        .retain(|state| state.user_id != update.user.id);
                }
            }
        }
        self.rebuild_structure(None);
        Change::Structure
    }
}
