use super::*;
use crate::discord::model;
use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VoiceActivity {
    Voice,
    Streaming,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveVoice {
    pub guild_id: u64,
    pub channel_id: u64,
    pub member_ids: Vec<u64>,
    pub friend_ids: Vec<u64>,
}

pub struct VoiceMember {
    pub user_id: u64,
    pub self_mute: bool,
    pub self_deaf: bool,
    pub mute: bool,
    pub deaf: bool,
    pub streaming: bool,
    pub video: bool,
}

impl Store {
    pub fn voice_started_at(&self, guild_id: u64, channel_id: u64) -> Option<SystemTime> {
        let seconds = *self.voice_start_times.get(&(guild_id, channel_id))?;
        UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
    }

    pub(super) fn channel_info(&mut self, info: model::ChannelInfo) -> Change {
        let mut change = Change::None;
        for channel in info.channels {
            if self.voice_start_time_update(info.guild_id, channel.id, channel.voice_start_time)
                == Change::VoiceTimers
            {
                change = Change::VoiceTimers;
            }
        }
        change
    }

    pub(super) fn voice_start_time_update(
        &mut self,
        guild_id: u64,
        channel_id: u64,
        start: Option<Option<u64>>,
    ) -> Change {
        if !self.guild_indices.contains_key(&guild_id) {
            return Change::None;
        }
        let Some(start) = start else {
            return Change::None;
        };
        let key = (guild_id, channel_id);
        let changed = match start {
            Some(start) => self.voice_start_times.insert(key, start) != Some(start),
            None => self.voice_start_times.remove(&key).is_some(),
        };
        if changed {
            Change::VoiceTimers
        } else {
            Change::None
        }
    }

    pub(super) fn apply_supplemental_voice(&mut self, update: &model::ReadySupplemental) -> bool {
        for user in &update.users {
            self.register_author(user);
        }
        let mut changed = false;
        for guild in &update.guilds {
            let Some(guild_id) = guild.get("id").and_then(model::id_value) else {
                continue;
            };
            let Some(states) = guild.get("voice_states") else {
                continue;
            };
            let mut states: Vec<model::VoiceState> = model::parse_entries(states.clone());
            for state in &mut states {
                state.guild_id = Some(guild_id);
                if let Some(member) = &mut state.member {
                    if member.id().is_none() {
                        member.user_id = Some(state.user_id);
                    }
                    let member = model::PartialMember::from(&*member);
                    self.upsert_member(guild_id, state.user_id, &member);
                }
            }
            if let Some(source) = &mut self.source
                && let Some(guild) = source.guilds.iter_mut().find(|guild| guild.id == guild_id)
            {
                guild.voice_states = states;
            }
            changed = true;
        }
        changed
    }

    pub(super) fn voice_state_update(&mut self, mut state: model::VoiceState) -> Change {
        let Some(guild_id) = state.guild_id else {
            return self.dm_voice_state_update(state);
        };
        if let Some(member) = &mut state.member {
            if member.id().is_none() {
                member.user_id = Some(state.user_id);
            }
            let member = model::PartialMember::from(&*member);
            self.upsert_member(guild_id, state.user_id, &member);
        }
        let Some(source) = &mut self.source else {
            return Change::None;
        };
        let Some(guild) = source.guilds.iter_mut().find(|guild| guild.id == guild_id) else {
            return Change::None;
        };
        let previous_channel = guild
            .voice_states
            .iter()
            .find(|existing| existing.user_id == state.user_id)
            .and_then(|existing| existing.channel_id);
        match state.channel_id {
            Some(_) => {
                if let Some(existing) = guild
                    .voice_states
                    .iter_mut()
                    .find(|existing| existing.user_id == state.user_id)
                {
                    *existing = state;
                } else {
                    guild.voice_states.push(state);
                }
            }
            None => {
                guild
                    .voice_states
                    .retain(|existing| existing.user_id != state.user_id);
            }
        }
        if let Some(channel_id) = previous_channel
            && !guild
                .voice_states
                .iter()
                .any(|state| state.channel_id == Some(channel_id))
        {
            self.voice_start_times.remove(&(guild_id, channel_id));
        }
        Change::Structure
    }

    pub fn voice_members(&self, guild_id: u64, channel_id: u64) -> Vec<VoiceMember> {
        if guild_id == 0 {
            return self
                .dm_call(channel_id)
                .map(|call| {
                    call.voice_states
                        .iter()
                        .map(|state| VoiceMember {
                            user_id: state.user_id,
                            self_mute: state.self_mute,
                            self_deaf: state.self_deaf,
                            mute: state.mute,
                            deaf: state.deaf,
                            streaming: state.self_stream,
                            video: state.self_video,
                        })
                        .collect()
                })
                .unwrap_or_default();
        }
        self.voice_members_matching(guild_id, |state| state.channel_id == Some(channel_id))
    }

    pub fn guild_voice_members(&self, guild_id: u64) -> Vec<VoiceMember> {
        let Some(guild) = self
            .source
            .as_ref()
            .and_then(|source| source.guilds.iter().find(|guild| guild.id == guild_id))
        else {
            return Vec::new();
        };
        self.voice_members_matching(guild_id, |state| self.voice_state_visible(guild, state))
    }

    pub fn active_voice(&self) -> Vec<ActiveVoice> {
        let Some(source) = self.source.as_ref() else {
            return Vec::new();
        };
        let friends: HashSet<u64> = self.friends().into_iter().collect();
        let mut channels: Vec<ActiveVoice> = Vec::new();
        for guild in &source.guilds {
            for state in &guild.voice_states {
                let Some(channel_id) = state.channel_id else {
                    continue;
                };
                if !self.voice_state_visible(guild, state) {
                    continue;
                }
                match channels
                    .iter_mut()
                    .find(|voice| voice.channel_id == channel_id)
                {
                    Some(voice) => voice.member_ids.push(state.user_id),
                    None => channels.push(ActiveVoice {
                        guild_id: guild.id,
                        channel_id,
                        member_ids: vec![state.user_id],
                        friend_ids: Vec::new(),
                    }),
                }
            }
        }
        for voice in &mut channels {
            voice.friend_ids = voice
                .member_ids
                .iter()
                .copied()
                .filter(|id| friends.contains(id))
                .collect();
            voice.member_ids.sort_by_key(|id| !friends.contains(id));
        }
        channels.retain(|voice| !voice.friend_ids.is_empty());
        channels.sort_by(|left, right| {
            right
                .member_ids
                .len()
                .cmp(&left.member_ids.len())
                .then(left.channel_id.cmp(&right.channel_id))
        });
        channels
    }

    pub fn user_voice_channel(&self, user_id: u64) -> Option<u64> {
        self.user_voice(user_id).map(|(_, channel_id)| channel_id)
    }

    pub fn user_voice(&self, user_id: u64) -> Option<(u64, u64)> {
        self.source.as_ref()?.guilds.iter().find_map(|guild| {
            guild
                .voice_states
                .iter()
                .find(|state| state.user_id == user_id && self.voice_state_visible(guild, state))
                .and_then(|state| Some((guild.id, state.channel_id?)))
        })
    }

    fn voice_state_visible(&self, guild: &model::Guild, state: &model::VoiceState) -> bool {
        let Some(channel_id) = state.channel_id else {
            return false;
        };
        guild
            .channels
            .iter()
            .flatten()
            .find(|channel| channel.id == channel_id)
            .is_none_or(|channel| crate::discord::can_view(guild, channel, self.current_user_id))
    }

    fn voice_members_matching(
        &self,
        guild_id: u64,
        include: impl Fn(&model::VoiceState) -> bool,
    ) -> Vec<VoiceMember> {
        let Some(guild) = self
            .source
            .as_ref()
            .and_then(|source| source.guilds.iter().find(|guild| guild.id == guild_id))
        else {
            return Vec::new();
        };
        let mut members: Vec<_> = guild
            .voice_states
            .iter()
            .filter(|state| include(state))
            .map(|state| VoiceMember {
                user_id: state.user_id,
                self_mute: state.self_mute,
                self_deaf: state.self_deaf,
                mute: state.mute,
                deaf: state.deaf,
                streaming: state.self_stream,
                video: state.self_video,
            })
            .collect();
        members.sort_by(|a, b| {
            let name = |member: &VoiceMember| {
                self.member_name(guild_id, member.user_id)
                    .or_else(|| self.user(member.user_id).map(|user| user.name.as_str()))
                    .filter(|name| !name.is_empty())
                    .map(str::to_lowercase)
                    .unwrap_or_else(|| member.user_id.to_string())
            };
            name(a)
                .cmp(&name(b))
                .then_with(|| a.user_id.cmp(&b.user_id))
        });
        members
    }

    pub fn guild_voice_activity(&self, guild_id: u64) -> Option<VoiceActivity> {
        let guild = self
            .source
            .as_ref()?
            .guilds
            .iter()
            .find(|guild| guild.id == guild_id)?;
        let mut states = guild
            .voice_states
            .iter()
            .filter(|state| self.voice_state_visible(guild, state))
            .peekable();
        states.peek()?;
        Some(if states.any(|state| state.self_stream) {
            VoiceActivity::Streaming
        } else {
            VoiceActivity::Voice
        })
    }

    pub fn can_connect_voice(&self, guild_id: u64, channel_id: u64) -> bool {
        if guild_id == 0 {
            return self
                .private_channel(channel_id)
                .is_some_and(|channel| !channel.message_request && !channel.spam);
        }
        let Some(guild) = self
            .source
            .as_ref()
            .and_then(|source| source.guilds.iter().find(|guild| guild.id == guild_id))
        else {
            return false;
        };
        let Some(channel) = guild
            .channels
            .iter()
            .flatten()
            .find(|channel| channel.id == channel_id && channel.kind == Some(2))
        else {
            return false;
        };
        crate::discord::channel_permissions(guild, channel, self.current_user_id)
            & crate::discord::CONNECT
            != 0
    }
}
