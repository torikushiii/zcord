use super::super::*;

impl Session {
    pub fn join_dm_call(&self, channel_id: u64) {
        self.join_voice(0, channel_id);
    }

    pub fn request_dm_call(&self, channel_id: u64) {
        self.command(commands::call_connect(channel_id));
    }

    pub async fn call_ringable(&self, channel_id: u64) -> Result<bool, Error> {
        self.rest(async |rest| rest.call_ringable(channel_id).await)
            .await
    }

    pub async fn ring_call(&self, channel_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.ring_call(channel_id).await)
            .await
    }

    pub async fn stop_ringing_call(&self, channel_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.stop_ringing_call(channel_id).await)
            .await
    }
}
