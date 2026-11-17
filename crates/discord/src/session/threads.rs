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

    pub async fn search_threads(
        &self,
        channel_id: u64,
        name: &str,
        tag_setting: Option<&str>,
    ) -> Result<ThreadPage, Error> {
        self.rest(async |rest| rest.search_threads(channel_id, name, tag_setting).await)
            .await
    }

    pub async fn join_thread(
        &self,
        thread_id: u64,
        location: model::ThreadFollowLocation,
    ) -> Result<(), Error> {
        self.rest(async |rest| rest.join_thread(thread_id, location).await)
            .await
    }

    pub async fn leave_thread(
        &self,
        thread_id: u64,
        location: model::ThreadFollowLocation,
    ) -> Result<(), Error> {
        self.rest(async |rest| rest.leave_thread(thread_id, location).await)
            .await
    }

    pub async fn create_standalone_thread(
        &self,
        channel_id: u64,
        name: &str,
        private: bool,
        announcement: bool,
        auto_archive_duration: Option<u16>,
    ) -> Result<model::Channel, Error> {
        self.rest(async |rest| {
            rest.create_standalone_thread(
                channel_id,
                name,
                private,
                announcement,
                auto_archive_duration,
            )
            .await
        })
        .await
    }

    pub async fn create_forum_post(
        &self,
        forum_id: u64,
        name: &str,
        content: &str,
        applied_tags: &[u64],
        auto_archive_duration: Option<u16>,
    ) -> Result<model::Channel, Error> {
        self.rest(async |rest| {
            rest.create_forum_post(forum_id, name, content, applied_tags, auto_archive_duration)
                .await
        })
        .await
    }

    pub async fn update_thread_settings(
        &self,
        thread_id: u64,
        settings: model::ThreadSettings,
        joined: bool,
    ) -> Result<(), Error> {
        self.rest(async |rest| {
            rest.update_thread_settings(thread_id, settings, joined)
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
        announcement: bool,
        auto_archive_duration: Option<u16>,
    ) -> Result<model::Channel, Error> {
        self.rest(async |rest| {
            rest.create_thread(
                channel_id,
                message_id,
                name,
                announcement,
                auto_archive_duration,
            )
            .await
        })
        .await
    }
}
