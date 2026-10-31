use super::super::*;

impl Session {
    pub async fn archived_threads(
        &self,
        channel_id: u64,
        offset: usize,
    ) -> Result<ThreadPage, Error> {
        self.rest(async |rest| rest.archived_threads(channel_id, offset).await)
            .await
    }

    pub async fn search_threads(&self, channel_id: u64, name: &str) -> Result<ThreadPage, Error> {
        self.rest(async |rest| rest.search_threads(channel_id, name).await)
            .await
    }

    pub async fn join_thread(&self, thread_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.join_thread(thread_id).await)
            .await
    }

    pub async fn leave_thread(&self, thread_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.leave_thread(thread_id).await)
            .await
    }

    pub async fn create_standalone_thread(
        &self,
        channel_id: u64,
        name: &str,
        private: bool,
    ) -> Result<model::Channel, Error> {
        self.rest(async |rest| {
            rest.create_standalone_thread(channel_id, name, private)
                .await
        })
        .await
    }

    pub async fn archive_thread(&self, thread_id: u64, archived: bool) -> Result<(), Error> {
        self.rest(async |rest| rest.archive_thread(thread_id, archived).await)
            .await
    }

    pub async fn delete_thread(&self, thread_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.delete_thread(thread_id).await)
            .await
    }

    pub async fn create_thread(
        &self,
        channel_id: u64,
        message_id: u64,
        name: &str,
    ) -> Result<model::Channel, Error> {
        self.rest(async |rest| rest.create_thread(channel_id, message_id, name).await)
            .await
    }
}
