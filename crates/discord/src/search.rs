use crate::model;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SearchScope {
    Guild(u64),
    Channel(u64),
    Dms,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SearchSort {
    #[default]
    Newest,
    Oldest,
    Relevant,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub content: String,
    pub author_ids: Vec<u64>,
    pub mentions: Vec<u64>,
    pub channel_ids: Vec<u64>,
    pub has: Vec<String>,
    pub min_id: Option<u64>,
    pub max_id: Option<u64>,
    pub pinned: Option<bool>,
    pub sort: SearchSort,
    pub offset: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ForumSort {
    #[default]
    Activity,
    Created,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForumQuery {
    pub sort: ForumSort,
    pub tags: Vec<u64>,
    pub match_all: bool,
    pub offset: usize,
}

pub enum SearchOutcome {
    Page(model::SearchPage),
    Indexing { retry_after: Duration },
}

#[derive(serde::Serialize)]
struct DmSearchBody<'a> {
    include_nsfw: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    channel_ids: Vec<String>,
    tabs: SearchTabs<'a>,
    track_exact_total_hits: bool,
}

#[derive(serde::Serialize)]
struct SearchTabs<'a> {
    messages: MessageSearchTab<'a>,
}

#[derive(serde::Serialize)]
struct MessageSearchTab<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    author_id: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    mentions: Vec<String>,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    has: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    min_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pinned: Option<bool>,
    sort_by: &'static str,
    sort_order: &'static str,
    limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<usize>,
}

pub(super) fn dm_search_body(query: &SearchQuery) -> impl serde::Serialize + '_ {
    DmSearchBody {
        include_nsfw: true,
        channel_ids: query.channel_ids.iter().map(u64::to_string).collect(),
        tabs: SearchTabs {
            messages: MessageSearchTab {
                content: (!query.content.is_empty()).then_some(query.content.as_str()),
                author_id: query.author_ids.iter().map(u64::to_string).collect(),
                mentions: query.mentions.iter().map(u64::to_string).collect(),
                has: &query.has,
                min_id: query.min_id.map(|id| id.to_string()),
                max_id: query.max_id.map(|id| id.to_string()),
                pinned: query.pinned,
                sort_by: match query.sort {
                    SearchSort::Relevant => "relevance",
                    SearchSort::Newest | SearchSort::Oldest => "timestamp",
                },
                sort_order: match query.sort {
                    SearchSort::Newest | SearchSort::Relevant => "desc",
                    SearchSort::Oldest => "asc",
                },
                limit: 25,
                offset: (query.offset != 0).then_some(query.offset),
            },
        },
        track_exact_total_hits: true,
    }
}

pub(super) fn search_params(
    scope: SearchScope,
    query: &SearchQuery,
) -> Vec<(&'static str, String)> {
    let mut params = Vec::new();
    if !query.content.is_empty() {
        params.push(("content", query.content.clone()));
    }
    params.extend(
        query
            .author_ids
            .iter()
            .map(|id| ("author_id", id.to_string())),
    );
    params.extend(query.mentions.iter().map(|id| ("mentions", id.to_string())));
    params.extend(
        query
            .channel_ids
            .iter()
            .map(|id| ("channel_id", id.to_string())),
    );
    params.extend(query.has.iter().map(|kind| ("has", kind.clone())));
    if let Some(min_id) = query.min_id {
        params.push(("min_id", min_id.to_string()));
    }
    if let Some(max_id) = query.max_id {
        params.push(("max_id", max_id.to_string()));
    }
    if let Some(pinned) = query.pinned {
        params.push(("pinned", pinned.to_string()));
    }
    params.extend([
        (
            "sort_by",
            match query.sort {
                SearchSort::Relevant => "relevance",
                SearchSort::Newest | SearchSort::Oldest => "timestamp",
            }
            .to_owned(),
        ),
        (
            "sort_order",
            match query.sort {
                SearchSort::Newest | SearchSort::Relevant => "desc",
                SearchSort::Oldest => "asc",
            }
            .to_owned(),
        ),
        ("limit", "25".to_owned()),
    ]);
    if query.offset != 0 {
        params.push(("offset", query.offset.to_string()));
    }
    if matches!(scope, SearchScope::Guild(_)) {
        params.push(("include_nsfw", "true".to_owned()));
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dm_search_body_serializes_filters_and_offset_in_messages_tab() {
        let query = SearchQuery {
            content: "hello & + /?".to_owned(),
            author_ids: vec![u64::MAX, 2],
            mentions: vec![3, 8],
            channel_ids: vec![4, 5],
            has: vec!["image".to_owned(), "snapshot".to_owned()],
            min_id: Some(6),
            max_id: Some(7),
            pinned: Some(false),
            sort: SearchSort::Oldest,
            offset: 50,
        };
        assert_eq!(
            serde_json::to_value(dm_search_body(&query)).unwrap(),
            serde_json::json!({
                "include_nsfw": true,
                "channel_ids": ["4", "5"],
                "track_exact_total_hits": true,
                "tabs": {"messages": {
                    "content": "hello & + /?",
                    "author_id": ["18446744073709551615", "2"],
                    "mentions": ["3", "8"],
                    "has": ["image", "snapshot"],
                    "min_id": "6",
                    "max_id": "7",
                    "pinned": false,
                    "sort_by": "timestamp",
                    "sort_order": "asc",
                    "limit": 25,
                    "offset": 50
                }}
            })
        );
    }

    #[test]
    fn dm_search_body_omits_empty_filters_and_uses_each_sort() {
        for (sort, sort_by, sort_order) in [
            (SearchSort::Newest, "timestamp", "desc"),
            (SearchSort::Oldest, "timestamp", "asc"),
            (SearchSort::Relevant, "relevance", "desc"),
        ] {
            let query = SearchQuery {
                sort,
                ..SearchQuery::default()
            };
            assert_eq!(
                serde_json::to_value(dm_search_body(&query)).unwrap(),
                serde_json::json!({
                    "include_nsfw": true,
                    "track_exact_total_hits": true,
                    "tabs": {"messages": {
                        "sort_by": sort_by, "sort_order": sort_order, "limit": 25
                    }}
                })
            );
        }
    }

    #[test]
    fn search_params_sort_relevance_descending() {
        let query = SearchQuery {
            sort: SearchSort::Relevant,
            ..SearchQuery::default()
        };
        let params = search_params(SearchScope::Channel(1), &query);
        assert!(params.contains(&("sort_by", "relevance".to_owned())));
        assert!(params.contains(&("sort_order", "desc".to_owned())));
    }

    #[test]
    fn search_params_include_repeated_filters_and_guild_options() {
        let query = SearchQuery {
            content: "hello world".to_owned(),
            author_ids: vec![1, 2],
            mentions: vec![3, 8],
            channel_ids: vec![4, 5],
            has: vec!["image".to_owned(), "snapshot".to_owned()],
            min_id: Some(6),
            max_id: Some(7),
            pinned: Some(false),
            sort: SearchSort::Oldest,
            offset: 50,
        };

        assert_eq!(
            search_params(SearchScope::Guild(9), &query),
            [
                ("content", "hello world".to_owned()),
                ("author_id", "1".to_owned()),
                ("author_id", "2".to_owned()),
                ("mentions", "3".to_owned()),
                ("mentions", "8".to_owned()),
                ("channel_id", "4".to_owned()),
                ("channel_id", "5".to_owned()),
                ("has", "image".to_owned()),
                ("has", "snapshot".to_owned()),
                ("min_id", "6".to_owned()),
                ("max_id", "7".to_owned()),
                ("pinned", "false".to_owned()),
                ("sort_by", "timestamp".to_owned()),
                ("sort_order", "asc".to_owned()),
                ("limit", "25".to_owned()),
                ("offset", "50".to_owned()),
                ("include_nsfw", "true".to_owned()),
            ]
        );
    }

    #[test]
    fn search_params_omit_empty_and_default_values() {
        let query = SearchQuery::default();
        assert_eq!(
            search_params(SearchScope::Channel(10), &query),
            [
                ("sort_by", "timestamp".to_owned()),
                ("sort_order", "desc".to_owned()),
                ("limit", "25".to_owned()),
            ]
        );
    }

    #[test]
    fn search_params_preserve_repeated_keys_and_encode_content_on_the_wire() {
        let query = SearchQuery {
            content: "hello & + /?".to_owned(),
            author_ids: vec![u64::MAX, 2],
            pinned: Some(true),
            ..SearchQuery::default()
        };
        let params = search_params(SearchScope::Channel(10), &query);
        let request = reqwest::Client::new()
            .get("https://discord.com/api/v9/channels/10/messages/search")
            .query(&params)
            .build()
            .unwrap();
        assert_eq!(
            request
                .url()
                .query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect::<Vec<_>>(),
            params
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect::<Vec<_>>()
        );
        assert!(params.contains(&("pinned", "true".to_owned())));
        assert!(!params.iter().any(|(key, _)| *key == "include_nsfw"));
    }
}
