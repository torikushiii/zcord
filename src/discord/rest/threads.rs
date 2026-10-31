use super::*;
use crate::discord::ThreadPage;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

const SEARCH_ATTEMPTS: usize = 4;
const SEARCH_RETRY: Duration = Duration::from_secs(2);

fn archived_threads_url(channel_id: u64, offset: usize) -> String {
    format!(
        "https://discord.com/api/v9/channels/{channel_id}/threads/search?archived=true&sort_by=last_message_time&sort_order=desc&limit=25&tag_setting=match_some&offset={offset}"
    )
}

fn search_threads_url(channel_id: u64, name: &str) -> String {
    format!(
        "https://discord.com/api/v9/channels/{channel_id}/threads/search?name={}&tag_setting=match_some",
        percent_encode(name)
    )
}

fn archived_threads_request(client: &Client, channel_id: u64, offset: usize) -> RequestBuilder {
    client.get(archived_threads_url(channel_id, offset))
}

fn search_threads_request(client: &Client, channel_id: u64, name: &str) -> RequestBuilder {
    client.get(search_threads_url(channel_id, name))
}

fn thread_member_url(thread_id: u64) -> String {
    format!(
        "https://discord.com/api/v9/channels/{thread_id}/thread-members/@me?location=Toolbar%20Overflow"
    )
}

fn join_thread_request(client: &Client, thread_id: u64) -> RequestBuilder {
    client.post(thread_member_url(thread_id))
}

fn leave_thread_request(client: &Client, thread_id: u64) -> RequestBuilder {
    client.delete(thread_member_url(thread_id))
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

fn create_thread_request<'a>(
    client: &Client,
    channel_id: u64,
    message_id: u64,
    name: &'a str,
) -> RequestBuilder {
    client
        .post(create_thread_url(channel_id, message_id))
        .json(&CreateThread {
            name,
            kind: 11,
            auto_archive_duration: 4320,
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

fn standalone_thread_request<'a>(
    client: &Client,
    channel_id: u64,
    name: &'a str,
    private: bool,
) -> RequestBuilder {
    client
        .post(standalone_thread_url(channel_id))
        .json(&StandaloneThread {
            name,
            kind: if private { 12 } else { 11 },
            auto_archive_duration: 4320,
            location: "Thread Browser Toolbar",
        })
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

    pub async fn search_threads(&self, channel_id: u64, name: &str) -> Result<ThreadPage, Error> {
        for attempt in 0..SEARCH_ATTEMPTS {
            let response = self
                .send(|| search_threads_request(&self.client, channel_id, name))
                .await?;
            if response.status().as_u16() != 202 {
                let value = response.json::<Value>().await.map_err(|_| Error::Decode)?;
                return parse_threads(value, channel_id);
            }
            if attempt + 1 == SEARCH_ATTEMPTS {
                break;
            }
            let retry = match response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<f64>().ok())
            {
                Some(value) => seconds(Some(value)).unwrap_or(SEARCH_RETRY),
                None => {
                    let value = response.json::<Value>().await.unwrap_or(Value::Null);
                    seconds(value.get("retry_after").and_then(Value::as_f64))
                        .unwrap_or(SEARCH_RETRY)
                }
            };
            sleep(retry).await;
        }
        Err(Error::Http(202))
    }

    pub async fn join_thread(&self, thread_id: u64) -> Result<(), Error> {
        self.send(|| join_thread_request(&self.client, thread_id))
            .await?;
        Ok(())
    }

    pub async fn leave_thread(&self, thread_id: u64) -> Result<(), Error> {
        self.send(|| leave_thread_request(&self.client, thread_id))
            .await?;
        Ok(())
    }

    pub async fn create_standalone_thread(
        &self,
        channel_id: u64,
        name: &str,
        private: bool,
    ) -> Result<model::Channel, Error> {
        let response = self
            .send(|| standalone_thread_request(&self.client, channel_id, name, private))
            .await?;
        response.json().await.map_err(|_| Error::Decode)
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
    ) -> Result<model::Channel, Error> {
        let response = self
            .send(|| create_thread_request(&self.client, channel_id, message_id, name))
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

        let request = search_threads_request(&client, 20, "a b").build().unwrap();
        assert_eq!(request.method(), reqwest::Method::GET);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/20/threads/search?name=a%20b&tag_setting=match_some"
        );
    }

    #[test]
    fn thread_member_and_create_requests_match_the_routes() {
        let client = Client::new();
        let request = join_thread_request(&client, 9).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::POST);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/9/thread-members/@me?location=Toolbar%20Overflow"
        );
        assert!(request.body().is_none());

        let request = leave_thread_request(&client, 9).build().unwrap();
        assert_eq!(request.method(), reqwest::Method::DELETE);
        assert_eq!(
            request.url().as_str(),
            "https://discord.com/api/v9/channels/9/thread-members/@me?location=Toolbar%20Overflow"
        );

        let request = create_thread_request(&client, 20, 30, "Plans")
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
    }

    #[test]
    fn standalone_thread_archive_and_delete_requests_match() {
        let client = Client::new();
        let request = standalone_thread_request(&client, 20, "Plans", false)
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

        let request = standalone_thread_request(&client, 20, "Secret", true)
            .build()
            .unwrap();
        assert_eq!(
            request.body().and_then(reqwest::Body::as_bytes),
            Some(
                br#"{"name":"Secret","type":12,"auto_archive_duration":4320,"location":"Thread Browser Toolbar"}"#
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
