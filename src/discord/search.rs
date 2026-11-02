use crate::discord::model;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SearchScope {
    Guild(u64),
    Channel(u64),
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

pub enum SearchOutcome {
    Page(model::SearchPage),
    Indexing { retry_after: Duration },
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
