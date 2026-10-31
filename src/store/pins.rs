use super::*;

impl Store {
    pub fn pins_unread(&self, channel_id: u64) -> bool {
        let Some(source) = &self.source else {
            return false;
        };
        let Some(pin) = self
            .source_channels
            .get(&channel_id)
            .and_then(|entry| entry.get(source))
            .and_then(|channel| channel.last_pin_timestamp.as_deref())
        else {
            return false;
        };
        match read_state(source, &self.read_states, channel_id)
            .and_then(|state| state.last_pin_timestamp.as_deref())
        {
            Some(read) => pin_is_newer(pin, read),
            None => true,
        }
    }

    pub fn mark_pins_read(&mut self, channel_id: u64) {
        let Some(pin) = self
            .source
            .as_ref()
            .and_then(|source| self.source_channels.get(&channel_id)?.get(source))
            .and_then(|channel| channel.last_pin_timestamp.clone())
        else {
            return;
        };
        if let Some(state) = self.read_state_mut(channel_id, 0) {
            state.last_pin_timestamp = Some(pin);
        }
    }

    pub fn can_read_history(&self, channel_id: u64) -> bool {
        if self.source.is_none() {
            return true;
        }
        if matches!(
            self.source_channels.get(&channel_id),
            Some(SourceChannel::Private(_))
        ) {
            return true;
        }
        self.permissions(channel_id)
            .is_some_and(|permissions| permissions & discord::READ_MESSAGE_HISTORY != 0)
    }

    pub(super) fn channel_pins_update(&mut self, update: model::ChannelPinsUpdate) -> Change {
        let Some(source) = &mut self.source else {
            return Change::None;
        };
        let Some(channel) = self
            .source_channels
            .get(&update.channel_id)
            .and_then(|entry| entry.get_mut(source))
        else {
            return Change::None;
        };
        channel.last_pin_timestamp = update.last_pin_timestamp;
        Change::Structure
    }
}

fn pin_is_newer(pin: &str, read: &str) -> bool {
    match (
        chrono::DateTime::parse_from_rfc3339(pin),
        chrono::DateTime::parse_from_rfc3339(read),
    ) {
        (Ok(pin), Ok(read)) => pin > read,
        _ => pin > read,
    }
}
