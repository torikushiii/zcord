use super::*;
use std::collections::HashSet;

pub struct SavedEntry {
    pub channel_id: u64,
    pub message_id: u64,
    pub guild_id: Option<u64>,
    pub due_ms: Option<i64>,
    pub message: Option<Message>,
}

#[derive(Default)]
pub(super) struct Saved {
    bookmark_ids: HashSet<u64>,
    reminders: Vec<SavedEntry>,
    bookmarks: Option<Vec<SavedEntry>>,
    bookmarks_cursor: Option<String>,
}

fn epoch_ms(value: Option<&str>) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value?)
        .ok()
        .map(|time| time.timestamp_millis())
}

impl Store {
    pub fn is_bookmarked(&self, message_id: u64) -> bool {
        self.saved.bookmark_ids.contains(&message_id)
    }

    pub fn reminder(&self, message_id: u64) -> Option<&SavedEntry> {
        self.saved
            .reminders
            .iter()
            .find(|entry| entry.message_id == message_id)
    }

    pub fn reminders(&self) -> &[SavedEntry] {
        &self.saved.reminders
    }

    pub fn bookmark_list(&self) -> Option<&[SavedEntry]> {
        self.saved.bookmarks.as_deref()
    }

    pub fn overdue_reminders(&self, now_ms: i64) -> usize {
        self.saved
            .reminders
            .iter()
            .filter(|entry| entry.due_ms.is_some_and(|due| due <= now_ms))
            .count()
    }

    pub fn next_reminder_due(&self, now_ms: i64) -> Option<i64> {
        self.saved
            .reminders
            .iter()
            .filter_map(|entry| entry.due_ms)
            .filter(|due| *due > now_ms)
            .min()
    }

    pub fn set_saved_messages(&mut self, saved: model::SavedMessages) {
        self.saved.bookmark_ids = saved.bookmark_ids.into_iter().collect();
        let reminders: Vec<_> = saved
            .reminders
            .into_iter()
            .map(|saved| self.saved_entry(saved))
            .collect();
        self.saved.reminders = reminders;
        self.sort_reminders();
    }

    pub fn set_bookmarks(&mut self, page: model::BookmarkPage) {
        self.saved.bookmarks = Some(Vec::new());
        self.add_bookmarks(page);
    }

    pub fn add_bookmarks(&mut self, page: model::BookmarkPage) {
        let last_saved_at = page
            .results
            .last()
            .and_then(|saved| saved.save_data.saved_at.clone());
        self.saved.bookmarks_cursor = last_saved_at.filter(|_| page.has_more);
        let entries: Vec<_> = page
            .results
            .into_iter()
            .map(|saved| self.saved_entry(saved))
            .collect();
        self.saved
            .bookmark_ids
            .extend(entries.iter().map(|entry| entry.message_id));
        let bookmarks = self.saved.bookmarks.get_or_insert_with(Vec::new);
        for entry in entries {
            if !bookmarks
                .iter()
                .any(|known| known.message_id == entry.message_id)
            {
                bookmarks.push(entry);
            }
        }
    }

    pub fn bookmarks_cursor(&self) -> Option<&str> {
        self.saved.bookmarks_cursor.as_deref()
    }

    pub fn message_saved(&mut self, saved: model::SavedMessage) -> Change {
        let entry = self.saved_entry(saved);
        self.forget_saved(entry.message_id);
        if entry.due_ms.is_some() {
            self.saved.reminders.push(entry);
            self.sort_reminders();
        } else {
            self.saved.bookmark_ids.insert(entry.message_id);
            if let Some(bookmarks) = &mut self.saved.bookmarks {
                bookmarks.insert(0, entry);
            }
        }
        Change::Saved
    }

    pub fn message_unsaved(&mut self, message_id: u64) -> Change {
        self.forget_saved(message_id);
        Change::Saved
    }

    fn forget_saved(&mut self, message_id: u64) {
        self.saved.bookmark_ids.remove(&message_id);
        self.saved
            .reminders
            .retain(|entry| entry.message_id != message_id);
        if let Some(bookmarks) = &mut self.saved.bookmarks {
            bookmarks.retain(|entry| entry.message_id != message_id);
        }
    }

    fn sort_reminders(&mut self) {
        self.saved
            .reminders
            .sort_by_key(|entry| entry.due_ms.unwrap_or(i64::MAX));
    }

    pub fn inbox_messages(&mut self, messages: &[model::Message]) -> Vec<Message> {
        messages
            .iter()
            .filter_map(|message| self.map_network_message(message, true))
            .collect()
    }

    fn saved_entry(&mut self, saved: model::SavedMessage) -> SavedEntry {
        let data = saved.save_data;
        let guild_id = data.guild_id.or_else(|| {
            self.guilds
                .iter()
                .find(|guild| guild.channel(data.channel_id).is_some())
                .map(|guild| guild.id)
        });
        let message = saved.message.and_then(|mut message| {
            if message.guild_id.is_none() {
                message.guild_id = guild_id;
            }
            self.map_network_message(&message, true)
        });
        SavedEntry {
            channel_id: data.channel_id,
            message_id: data.message_id,
            guild_id,
            due_ms: epoch_ms(data.due_at.as_deref()),
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn saved(channel_id: u64, message_id: u64, due_at: Option<&str>) -> model::SavedMessage {
        serde_json::from_value(json!({
            "save_data": {"channel_id": channel_id.to_string(), "message_id": message_id.to_string(),
                "saved_at": "2026-11-06T20:11:29.594541+00:00", "due_at": due_at},
        }))
        .unwrap()
    }

    #[test]
    fn bookmarks_and_reminders_move_between_lists() {
        let mut store = Store::seed();
        let guild = &store.guilds[0];
        let (guild_id, channel_id) = (guild.id, guild.all_channels().next().unwrap().id);
        store.set_saved_messages(
            serde_json::from_value(json!({"reminders": [], "bookmark_ids": ["10"]})).unwrap(),
        );
        assert!(store.is_bookmarked(10));

        store.message_saved(saved(channel_id, 10, Some("2026-11-07T11:22:20.836Z")));
        assert!(!store.is_bookmarked(10));
        assert_eq!(store.reminders().len(), 1);
        assert_eq!(store.reminder(10).unwrap().guild_id, Some(guild_id));

        store.message_saved(saved(channel_id, 11, Some("2026-11-07T10:00:00Z")));
        assert_eq!(
            store
                .reminders()
                .iter()
                .map(|entry| entry.message_id)
                .collect::<Vec<_>>(),
            [11, 10]
        );
        let due = |value: &str| epoch_ms(Some(value)).unwrap();
        assert_eq!(store.overdue_reminders(due("2026-11-07T10:30:00Z")), 1);
        assert_eq!(
            store.next_reminder_due(due("2026-11-07T10:30:00Z")),
            Some(due("2026-11-07T11:22:20.836Z"))
        );

        store.message_unsaved(10);
        assert!(store.reminder(10).is_none());
        store.message_saved(saved(channel_id, 12, None));
        assert!(store.is_bookmarked(12));
    }

    #[test]
    fn bookmark_pages_append_and_stop_when_discord_has_no_more() {
        let mut store = Store::seed();
        let channel_id = store.guilds[0].all_channels().next().unwrap().id;
        let page = |ids: &[u64], has_more: bool| model::BookmarkPage {
            results: ids.iter().map(|id| saved(channel_id, *id, None)).collect(),
            has_more,
        };
        store.set_bookmarks(page(&[1, 2], true));
        assert_eq!(
            store.bookmarks_cursor(),
            Some("2026-11-06T20:11:29.594541+00:00")
        );
        store.add_bookmarks(page(&[2, 3], false));
        let ids: Vec<u64> = store
            .bookmark_list()
            .unwrap()
            .iter()
            .map(|entry| entry.message_id)
            .collect();
        assert_eq!(ids, [1, 2, 3]);
        assert_eq!(store.bookmarks_cursor(), None);
        store.set_bookmarks(page(&[4], false));
        assert_eq!(store.bookmark_list().unwrap().len(), 1);
    }
}
