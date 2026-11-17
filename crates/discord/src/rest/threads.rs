use super::*;
use crate::ThreadPage;
use serde::{Deserialize, Serialize};

const DEFAULT_AUTO_ARCHIVE_MINUTES: u16 = 4320;
const MATCH_SOME: &str = "match_some";
const SILENT_PREFIX: &str = "@silent";
pub(super) const SUPPRESS_NOTIFICATIONS: u64 = 1 << 12;

fn archived_threads_url(channel_id: u64, offset: usize) -> String {
    format!(
        "https://discord.com/api/v9/channels/{channel_id}/threads/search?archived=true&sort_by=last_message_time&sort_order=desc&limit=25&tag_setting={MATCH_SOME}&offset={offset}"
    )
}

fn search_threads_url(channel_id: u64, name: &str, tag_setting: &str) -> String {
    format!(
        "https://discord.com/api/v9/channels/{channel_id}/threads/search?name={}&tag_setting={}",
        percent_encode(name),
        percent_encode(tag_setting)
    )
}

fn archived_threads_request(client: &Client, channel_id: u64, offset: usize) -> RequestBuilder {
    client.get(archived_threads_url(channel_id, offset))
}

fn search_threads_request(
    client: &Client,
    channel_id: u64,
    name: &str,
    tag_setting: &str,
) -> RequestBuilder {
    client.get(search_threads_url(channel_id, name, tag_setting))
}

fn thread_member_url(thread_id: u64, location: &str) -> String {
    format!(
        "https://discord.com/api/v9/channels/{thread_id}/thread-members/@me?location={}",
        location.replace(' ', "%20")
    )
}

const CHANGE_NOTIFICATION_SETTINGS: &str = "Change Notification Settings";

fn follow_location(location: model::ThreadFollowLocation) -> &'static str {
    match location {
        model::ThreadFollowLocation::ToolbarOverflow => "Toolbar Overflow",
        model::ThreadFollowLocation::ForumToolbar => "Forum Toolbar",
    }
}

fn join_thread_request(client: &Client, thread_id: u64, location: &str) -> RequestBuilder {
    client.post(thread_member_url(thread_id, location))
}

fn leave_thread_request(client: &Client, thread_id: u64, location: &str) -> RequestBuilder {
    client.delete(thread_member_url(thread_id, location))
}

fn thread_kind(private: bool, announcement: bool) -> u8 {
    if private {
        12
    } else if announcement {
        10
    } else {
        11
    }
}

fn create_thread_url(channel_id: u64, message_id: u64) -> String {
    format!("https://discord.com/api/v9/channels/{channel_id}/messages/{message_id}/threads")
}

#[derive(Serialize)]
struct CreateThread<'a> {
    name: &'a str,
    #[serde(rename = "type")]
    kind: u8,
    auto_archive_duration: u16,
    location: &'static str,
}

fn create_thread_request(
    client: &Client,
    channel_id: u64,
    message_id: u64,
    name: &str,
    announcement: bool,
    auto_archive_duration: Option<u16>,
) -> RequestBuilder {
    client
        .post(create_thread_url(channel_id, message_id))
        .json(&CreateThread {
            name,
            kind: thread_kind(false, announcement),
            auto_archive_duration: auto_archive_duration.unwrap_or(DEFAULT_AUTO_ARCHIVE_MINUTES),
            location: "Message",
        })
}

fn standalone_thread_url(channel_id: u64) -> String {
    format!("https://discord.com/api/v9/channels/{channel_id}/threads")
}

#[derive(Serialize)]
struct StandaloneThread<'a> {
    name: &'a str,
    #[serde(rename = "type")]
    kind: u8,
    auto_archive_duration: u16,
    location: &'static str,
}

fn standalone_thread_request(
    client: &Client,
    channel_id: u64,
    name: &str,
    private: bool,
    announcement: bool,
    auto_archive_duration: Option<u16>,
) -> RequestBuilder {
    client
        .post(standalone_thread_url(channel_id))
        .json(&StandaloneThread {
            name,
            kind: thread_kind(private, announcement),
            auto_archive_duration: auto_archive_duration.unwrap_or(DEFAULT_AUTO_ARCHIVE_MINUTES),
            location: "Thread Browser Toolbar",
        })
}

#[derive(Serialize)]
struct ForumPost<'a> {
    name: &'a str,
    auto_archive_duration: u16,
    applied_tags: Vec<String>,
    message: ForumPostMessage<'a>,
}

#[derive(Serialize)]
struct ForumPostMessage<'a> {
    content: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    flags: Option<u64>,
}

pub(super) fn strip_silent(content: &str) -> Option<&str> {
    let rest = content.strip_prefix(SILENT_PREFIX)?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

fn create_forum_post_request<'a>(
    client: &Client,
    forum_id: u64,
    name: &'a str,
    content: &'a str,
    applied_tags: &'a [u64],
    auto_archive_duration: Option<u16>,
) -> RequestBuilder {
    let silent = strip_silent(content);
    let message = ForumPostMessage {
        content: silent.unwrap_or(content),
        flags: silent.map(|_| SUPPRESS_NOTIFICATIONS),
    };
    client
        .post(format!(
            "https://discord.com/api/v9/channels/{forum_id}/threads?use_nested_fields=true"
        ))
        .json(&ForumPost {
            name,
            auto_archive_duration: auto_archive_duration.unwrap_or(DEFAULT_AUTO_ARCHIVE_MINUTES),
            applied_tags: applied_tags.iter().map(u64::to_string).collect(),
            message,
        })
}

fn thread_settings_request(
    client: &Client,
    thread_id: u64,
    settings: model::ThreadSettings,
) -> RequestBuilder {
    client
        .patch(format!(
            "https://discord.com/api/v9/channels/{thread_id}/thread-members/@me/settings"
        ))
        .json(&settings)
}

#[derive(Serialize)]
struct ArchivedThread {
    archived: bool,
}

fn archive_thread_request(client: &Client, thread_id: u64, archived: bool) -> RequestBuilder {
    client
        .patch(format!("https://discord.com/api/v9/channels/{thread_id}"))
        .json(&ArchivedThread { archived })
}

fn delete_thread_request(client: &Client, thread_id: u64) -> RequestBuilder {
    client.delete(format!("https://discord.com/api/v9/channels/{thread_id}"))
}

#[derive(Deserialize)]
struct ThreadsReply {
    #[serde(deserialize_with = "model::entries")]
    threads: Vec<model::Channel>,
    has_more: bool,
}

fn parse_threads(value: Value, channel_id: u64) -> Result<ThreadPage, Error> {
    let reply: ThreadsReply = serde_json::from_value(value).map_err(|_| Error::Decode)?;
    let mut threads = reply.threads;
    threads.retain(|thread| thread.parent_id.flatten() == Some(channel_id));
    Ok(ThreadPage {
        threads,
        has_more: reply.has_more,
    })
}

impl Rest {
    pub async fn archived_threads(
        &self,
        channel_id: u64,
        offset: usize,
    ) -> Result<ThreadPage, Error> {
        let response = self
            .send(|| archived_threads_request(&self.client, channel_id, offset))
            .await?;
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        parse_threads(value, channel_id)
    }

    pub async fn search_threads(
        &self,
        channel_id: u64,
        name: &str,
        tag_setting: Option<&str>,
    ) -> Result<ThreadPage, Error> {
        let tag_setting = tag_setting.unwrap_or(MATCH_SOME);
        let response = self
            .send(|| search_threads_request(&self.client, channel_id, name, tag_setting))
            .await?;
        if response.status() == reqwest::StatusCode::ACCEPTED {
            return Err(Error::Http(202));
        }
        let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
        parse_threads(value, channel_id)
    }

    pub async fn join_thread(
        &self,
        thread_id: u64,
        location: model::ThreadFollowLocation,
    ) -> Result<(), Error> {
        self.send(|| join_thread_request(&self.client, thread_id, follow_location(location)))
            .await?;
        Ok(())
    }

    pub async fn leave_thread(
        &self,
        thread_id: u64,
        location: model::ThreadFollowLocation,
    ) -> Result<(), Error> {
        self.send(|| leave_thread_request(&self.client, thread_id, follow_location(location)))
            .await?;
        Ok(())
    }

    pub async fn create_standalone_thread(
        &self,
        channel_id: u64,
        name: &str,
        private: bool,
        announcement: bool,
        auto_archive_duration: Option<u16>,
    ) -> Result<model::Channel, Error> {
        let response = self
            .send(|| {
                standalone_thread_request(
                    &self.client,
                    channel_id,
                    name,
                    private,
                    announcement,
                    auto_archive_duration,
                )
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn create_forum_post(
        &self,
        forum_id: u64,
        name: &str,
        content: &str,
        applied_tags: &[u64],
        auto_archive_duration: Option<u16>,
    ) -> Result<model::Channel, Error> {
        let response = self
            .send(|| {
                create_forum_post_request(
                    &self.client,
                    forum_id,
                    name,
                    content,
                    applied_tags,
                    auto_archive_duration,
                )
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }

    pub async fn update_thread_settings(
        &self,
        thread_id: u64,
        settings: model::ThreadSettings,
        joined: bool,
    ) -> Result<(), Error> {
        if !joined {
            self.send(|| {
                join_thread_request(&self.client, thread_id, CHANGE_NOTIFICATION_SETTINGS)
            })
            .await?;
        }
        self.send(|| thread_settings_request(&self.client, thread_id, settings))
            .await?;
        Ok(())
    }

    pub async fn archive_thread(&self, thread_id: u64, archived: bool) -> Result<(), Error> {
        self.send(|| archive_thread_request(&self.client, thread_id, archived))
            .await?;
        Ok(())
    }

    pub async fn delete_thread(&self, thread_id: u64) -> Result<(), Error> {
        self.send(|| delete_thread_request(&self.client, thread_id))
            .await?;
        Ok(())
    }

    pub async fn create_thread(
        &self,
        channel_id: u64,
        message_id: u64,
        name: &str,
        announcement: bool,
        auto_archive_duration: Option<u16>,
    ) -> Result<model::Channel, Error> {
        let response = self
            .send(|| {
                create_thread_request(
                    &self.client,
                    channel_id,
                    message_id,
                    name,
                    announcement,
                    auto_archive_duration,
                )
            })
            .await?;
        response.json().await.map_err(|_| Error::Decode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Client;

    #[test]
    fn thread_search_requests_match_the_client_routes() {
        let client = Client::new();
        let request = archived_threads_request(&client, 20, 50).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::GET);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/20/threads/search?archived=true&sort_by=last_message_time&sort_order=desc&limit=25&tag_setting=match_some&offset=50"
        );
        assert!(request.body().is_none());

        let request = search_threads_request(&client, 20, "a b", MATCH_SOME)
            .build()
            .unwrap();
        assert_eq!(request.method(), reqwest::Method::GET);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/20/threads/search?name=a%20b&tag_setting=match_some"
        );

        let request = search_threads_request(&client, 20, "a", "match_all")
            .build()
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/20/threads/search?name=a&tag_setting=match_all"
        );
    }

    #[test]
    fn thread_member_and_create_requests_match_the_routes() {
        let client = Client::new();
        let toolbar = follow_location(model::ThreadFollowLocation::ToolbarOverflow);
        let request = join_thread_request(&client, 9, toolbar).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/9/thread-members/@me?location=Toolbar%20Overflow"
        );
        assert!(request.body().is_none());

        let request = leave_thread_request(&client, 9, toolbar).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::DELETE);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/9/thread-members/@me?location=Toolbar%20Overflow"
        );

        let forum = follow_location(model::ThreadFollowLocation::ForumToolbar);
        for request in [
            join_thread_request(&client, 9, forum),
            leave_thread_request(&client, 9, forum),
        ] {
            assert_eq!(
                request.build().unwrap().url().as_str(),
                "https://discord.com/api/v9/channels/9/thread-members/@me?location=Forum%20Toolbar"
            );
        }

        let request = join_thread_request(&client, 9, CHANGE_NOTIFICATION_SETTINGS)
            .build()
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/9/thread-members/@me?location=Change%20Notification%20Settings"
        );

        let request = create_thread_request(&client, 20, 30, "Plans", false, None)
            .build()
            .unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/20/messages/30/threads"
        );
        assert_eq!(
            request.body().and_then(reqwest::Body::as_bytes),
            Some(
                br#"{"name":"Plans","type":11,"auto_archive_duration":4320,"location":"Message"}"#
                    .as_slice()
            )
        );

        let request = create_thread_request(&client, 20, 30, "News", true, Some(60))
            .build()
            .unwrap();
        assert_eq!(
            request.body().and_then(reqwest::Body::as_bytes),
            Some(
                br#"{"name":"News","type":10,"auto_archive_duration":60,"location":"Message"}"#
                    .as_slice()
            )
        );
    }

    #[test]
    fn standalone_thread_archive_and_delete_requests_match() {
        let client = Client::new();
        let request = standalone_thread_request(&client, 20, "Plans", false, false, None)
            .build()
            .unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/20/threads"
        );
        assert_eq!(
            request.body().and_then(reqwest::Body::as_bytes),
            Some(
                br#"{"name":"Plans","type":11,"auto_archive_duration":4320,"location":"Thread Browser Toolbar"}"#
                    .as_slice()
            )
        );

        let request = standalone_thread_request(&client, 20, "Secret", true, false, Some(10080))
            .build()
            .unwrap();
        assert_eq!(
            request.body().and_then(reqwest::Body::as_bytes),
            Some(
                br#"{"name":"Secret","type":12,"auto_archive_duration":10080,"location":"Thread Browser Toolbar"}"#
                    .as_slice()
            )
        );

        let request = standalone_thread_request(&client, 20, "News", false, true, None)
            .build()
            .unwrap();
        assert_eq!(
            request.body().and_then(reqwest::Body::as_bytes),
            Some(
                br#"{"name":"News","type":10,"auto_archive_duration":4320,"location":"Thread Browser Toolbar"}"#
                    .as_slice()
            )
        );

        let request = archive_thread_request(&client, 9, true).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::PATCH);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/9"
        );
        assert_eq!(
            request.body().and_then(reqwest::Body::as_bytes),
            Some(br#"{"archived":true}"#.as_slice())
        );

        let request = delete_thread_request(&client, 9).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::DELETE);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/9"
        );
        assert!(request.body().is_none());
    }

    #[test]
    fn forum_post_and_thread_settings_requests_match() {
        let client = Client::new();
        let request = create_forum_post_request(&client, 20, "Plans", "hello", &[1, 2], None)
            .build()
            .unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/20/threads?use_nested_fields=true"
        );
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "name": "Plans",
                "auto_archive_duration": 4320,
                "applied_tags": ["1", "2"],
                "message": {"content": "hello"}
            })
        );

        let request =
            create_forum_post_request(&client, 20, "Quiet", "@silent  shh ", &[], Some(1440))
                .build()
                .unwrap();
        assert_eq!(
            request.body().and_then(reqwest::Body::as_bytes),
            Some(
                br#"{"name":"Quiet","auto_archive_duration":1440,"applied_tags":[],"message":{"content":"shh","flags":4096}}"#
                    .as_slice()
            )
        );
        let request = create_forum_post_request(&client, 20, "Loud", "@silently", &[], None)
            .build()
            .unwrap();
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body["message"], serde_json::json!({"content": "@silently"}));

        let request = thread_settings_request(
            &client,
            9,
            model::ThreadSettings {
                muted: Some(true),
                flags: None,
            },
        )
        .build()
        .unwrap();
        assert_eq!(request.method(), reqwest::Method::PATCH);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/9/thread-members/@me/settings"
        );
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body, serde_json::json!({"muted": true}));
    }

    #[test]
    fn thread_reply_keeps_only_the_parent_channel() {
        let page = parse_threads(
            serde_json::json!({
                "threads": [
                    {"id": "9", "parent_id": "20", "type": 11, "name": "kept",
                     "thread_metadata": {"archived": true, "archive_timestamp": "2026-09-10T12:00:00Z"}},
                    {"id": "10", "parent_id": "21", "type": 11, "name": "other"}
                ],
                "members": [],
                "has_more": true,
                "total_results": 2
            }),
            20,
        )
        .unwrap();
        assert!(page.has_more);
        assert_eq!(page.threads.len(), 1);
        assert_eq!(page.threads[0].id, 9);
        assert_eq!(
            page.threads[0]
                .thread_metadata
                .as_ref()
                .and_then(|metadata| metadata.archive_timestamp.as_deref()),
            Some("2026-09-10T12:00:00Z")
        );
        assert!(matches!(
            parse_threads(serde_json::json!({}), 20),
            Err(Error::Decode)
        ));
    }
}
