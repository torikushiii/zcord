use super::*;
use crate::discord::model;

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
            return Change::None;
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
        Change::Structure
    }

    pub fn voice_members(&self, guild_id: u64, channel_id: u64) -> Vec<VoiceMember> {
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
            .filter(|state| state.channel_id == Some(channel_id))
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

    pub fn can_connect_voice(&self, guild_id: u64, channel_id: u64) -> bool {
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
