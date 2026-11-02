use super::super::*;

impl Session {
    pub fn join_dm_call(&self, channel_id: u64) {
        self.join_voice(0, channel_id);
    }

    /// Requests existing private-call state without joining or ringing.
    pub fn request_dm_call(&self, channel_id: u64) {
        self.command(commands::Command {
            payload: serde_json::json!({"op": 13, "d": {"channel_id": channel_id.to_string()}}),
            members: false,
            presence_revision: None,
            completion: None,
        });
    }

    pub async fn ring_call(&self, channel_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.ring_call(channel_id).await)
            .await
    }

    pub async fn stop_ringing_call(&self, channel_id: u64, user_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.stop_ringing_call(channel_id, user_id).await)
            .await
    }
}
