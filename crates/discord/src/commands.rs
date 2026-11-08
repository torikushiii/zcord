use serde_json::{Value, json};
use std::collections::VecDeque;
use tokio::time::{Duration, Instant};

pub(super) struct Command {
    pub payload: Value,
    pub members: bool,
    pub presence_revision: Option<u64>,
    pub completion: Option<tokio::sync::oneshot::Sender<Result<(), super::Error>>>,
}

impl Command {
    pub fn complete(self, result: Result<(), super::Error>) {
        if let Some(completion) = self.completion {
            let _ = completion.send(result);
        }
    }
}

pub(super) fn presence(payload: Value, revision: u64) -> Command {
    Command {
        payload,
        members: false,
        presence_revision: Some(revision),
        completion: None,
    }
}

pub(super) fn voice_state(payload: Value) -> Command {
    Command {
        payload,
        members: false,
        presence_revision: None,
        completion: None,
    }
}

pub(super) fn voice_start_times(guild_id: u64) -> Command {
    Command {
        members: false,
        presence_revision: None,
        completion: None,
        payload: json!({"op":43,"d":{
            "guild_id":guild_id.to_string(),"fields":["voice_start_time"]
        }}),
    }
}

pub(super) fn subscription(guild_id: u64, channel_id: u64, ranges: &[(u32, u32)]) -> Command {
    let ranges: Vec<_> = ranges
        .iter()
        .take(5)
        .filter(|(start, end)| start <= end)
        .map(|&(start, end)| [start, end])
        .collect();
    Command {
        members: false,
        presence_revision: None,
        completion: None,
        payload: json!({"op":37,"d":{"subscriptions":{
            guild_id.to_string(): {"typing":true,"activities":true,"threads":true,
                "member_updates":false,"channels":{channel_id.to_string():ranges}}
        }}}),
    }
}

pub(super) fn members(guild_id: u64, user_ids: &[u64]) -> Vec<Command> {
    user_ids
        .chunks(100)
        .map(|chunk| Command {
            members: true,
            presence_revision: None,
            completion: None,
            payload: json!({"op":8,"d":{"guild_id":guild_id.to_string(),
            "user_ids":chunk.iter().map(u64::to_string).collect::<Vec<_>>(),"presences":false}}),
        })
        .collect()
}

pub(super) fn query_members(guild_id: u64, query: &str, limit: u32) -> Command {
    Command {
        members: true,
        presence_revision: None,
        completion: None,
        payload: json!({"op":8,"d":{
            "guild_id":[guild_id.to_string()],"query":query,"limit":limit,"presences":false
        }}),
    }
}

pub(super) struct Commands {
    queue: VecDeque<Command>,
    tokens: f64,
    refilled_at: Instant,
    next_members: Instant,
    pub presence_after: Instant,
}

impl Commands {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            tokens: 10.0,
            refilled_at: Instant::now(),
            next_members: Instant::now(),
            presence_after: Instant::now(),
        }
    }

    pub fn push(&mut self, ready: bool, mut command: Command) {
        if ready {
            if let Some(revision) = command.presence_revision
                && let Some(index) = self
                    .queue
                    .iter()
                    .position(|queued| queued.presence_revision.is_some())
            {
                let queued = &mut self.queue[index];
                if queued
                    .presence_revision
                    .is_some_and(|queued| queued > revision)
                {
                    if queued.completion.is_none() {
                        queued.completion = command.completion.take();
                    }
                    command.complete(Err(super::Error::SettingsOutdated));
                    return;
                }
                let mut previous = self.queue.remove(index).expect("queued presence");
                if command.completion.is_none() {
                    command.completion = previous.completion.take();
                }
                previous.complete(Err(super::Error::SettingsOutdated));
            }
            self.queue.push_back(command);
        } else {
            log::debug!("Dropping gateway command before READY/RESUMED");
        }
    }

    pub fn hand_off(&mut self, mut command: Command) -> Option<Command> {
        let Some(queued) = self
            .queue
            .iter_mut()
            .find(|queued| queued.presence_revision.is_some())
        else {
            return Some(command);
        };
        if queued.completion.is_none() {
            queued.completion = command.completion.take();
        }
        command.complete(Err(super::Error::SettingsOutdated));
        None
    }

    pub fn record_send(&mut self, now: Instant) {
        self.refill(now);
        self.tokens = (self.tokens - 1.0).max(0.0);
    }

    fn refill(&mut self, now: Instant) {
        self.tokens = (self.tokens
            + now.duration_since(self.refilled_at).as_secs_f64() * (100.0 / 60.0))
            .min(10.0);
        self.refilled_at = now;
    }

    pub fn next_send(&mut self) -> Option<Instant> {
        let now = Instant::now();
        self.refill(now);
        self.next_command(now).map(|(_, next)| next)
    }

    fn next_command(&self, now: Instant) -> Option<(usize, Instant)> {
        // Burst 10 + refill 100/minute leaves 10 of Discord's 120 slots for heartbeats.
        // https://docs.discord.food/gateway/using-gateway#rate-limiting
        let shared = now + Duration::from_secs_f64((1.0 - self.tokens).max(0.0) * 0.6);
        self.queue
            .iter()
            .enumerate()
            .map(|(index, command)| {
                let mut next = shared;
                if command.members {
                    next = next.max(self.next_members);
                }
                if command.presence_revision.is_some() {
                    next = next.max(self.presence_after);
                }
                (index, next)
            })
            .min_by_key(|(_, next)| *next)
    }

    pub fn pop(&mut self) -> Option<Command> {
        let now = Instant::now();
        self.refill(now);
        let (index, _) = self.next_command(now)?;
        let command = self.queue.remove(index)?;
        if command.members {
            self.next_members = now + Duration::from_millis(300);
        }
        if command.presence_revision.is_some() {
            self.presence_after = now + Duration::from_secs(5);
        }
        self.record_send(now);
        Some(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_member_queries_use_op_8_and_the_member_rate_limit() {
        let command = query_members(123, "Carol M", 10);
        assert_eq!(
            command.payload,
            json!({"op":8,"d":{
                "guild_id":["123"],"query":"Carol M","limit":10,"presences":false
            }})
        );
        assert!(command.members);
        assert!(command.presence_revision.is_none());
        assert!(command.completion.is_none());
        let mut queue = Commands::new();
        queue.push(false, query_members(123, "", 0));
        assert!(queue.next_send().is_none());
        queue.push(true, command);
        queue.push(true, members(123, &[456]).remove(0));
        assert_eq!(queue.pop().unwrap().payload["d"]["query"], "Carol M");
        assert_eq!(
            queue.next_members.duration_since(queue.refilled_at),
            Duration::from_millis(300)
        );
    }

    #[test]
    fn delayed_presence_does_not_block_voice_members_or_subscriptions() {
        let mut queue = Commands::new();
        queue.presence_after = Instant::now() + Duration::from_secs(5);
        queue.push(true, presence(json!({"op":3}), 1));
        queue.push(true, voice_state(json!({"op":4,"d":{"channel_id":null}})));
        queue.push(true, subscription(1, 2, &[]));
        queue.push(true, members(1, &[2]).remove(0));
        let now = Instant::now();
        assert!(queue.next_send().unwrap() <= Instant::now());
        assert_eq!(queue.pop().unwrap().payload["op"], 4);
        assert_eq!(queue.pop().unwrap().payload["op"], 37);
        assert_eq!(queue.pop().unwrap().payload["op"], 8);
        assert!(queue.next_send().unwrap() >= now + Duration::from_secs(4));
        queue.presence_after = Instant::now();
        let command = queue.pop().unwrap();
        assert_eq!(command.presence_revision, Some(1));
        assert!(queue.presence_after >= Instant::now() + Duration::from_secs(4));
    }

    #[test]
    fn presence_coalescing_keeps_the_newest_revision_and_carries_superseded_completions() {
        let mut queue = Commands::new();
        let (done, mut result) = tokio::sync::oneshot::channel();
        let mut command = presence(json!({"op":3}), 1);
        command.completion = Some(done);
        queue.push(true, command);
        queue.push(true, presence(json!({"op":3}), 2));
        queue.push(true, presence(json!({"op":3}), 1));
        assert_eq!(queue.queue.len(), 1);
        assert!(matches!(
            result.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        let command = queue.pop().unwrap();
        assert_eq!(command.presence_revision, Some(2));
        command.complete(Ok(()));
        assert!(matches!(result.try_recv(), Ok(Ok(()))));

        let (done, mut result) = tokio::sync::oneshot::channel();
        let mut stale = presence(json!({"op":3}), 3);
        stale.completion = Some(done);
        queue.push(true, presence(json!({"op":3}), 4));
        assert!(queue.hand_off(stale).is_none());
        queue.pop().unwrap().complete(Ok(()));
        assert!(matches!(result.try_recv(), Ok(Ok(()))));
        assert!(queue.hand_off(presence(json!({"op":3}), 5)).is_some());
    }

    #[test]
    fn completion_means_socket_write_not_enqueue_and_drops_are_not_success() {
        let mut queue = Commands::new();
        let (done, mut result) = tokio::sync::oneshot::channel();
        let mut command = presence(json!({"op":3}), 3);
        command.completion = Some(done);
        queue.push(true, command);
        queue.push(true, presence(json!({"op":3}), 3));
        assert!(matches!(
            result.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        queue.pop().unwrap().complete(Ok(()));
        assert!(matches!(result.try_recv(), Ok(Ok(()))));
        let (done, mut result) = tokio::sync::oneshot::channel();
        let mut command = presence(json!({"op":3}), 4);
        command.completion = Some(done);
        queue.push(false, command);
        assert!(matches!(
            result.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ));
    }

    #[test]
    fn voice_start_times_requests_server_session_timestamps() {
        let command = voice_start_times(123);
        assert!(!command.members);
        assert_eq!(
            command.payload,
            json!({"op":43,"d":{"guild_id":"123","fields":["voice_start_time"]}})
        );
        let mut queue = Commands::new();
        queue.push(true, command);
        assert_eq!(queue.pop().unwrap().payload["op"], 43);
    }

    #[test]
    fn payloads_chunking_and_not_ready_drop() {
        let ranges = [
            (0, 99),
            (100, 199),
            (200, 299),
            (300, 399),
            (400, 499),
            (500, 599),
        ];
        let payload = subscription(1, 2, &ranges).payload;
        assert_eq!(
            payload,
            json!({"op":37,"d":{"subscriptions":{"1":{
                "typing":true,"activities":true,"threads":true,"member_updates":false,
                "channels":{"2":[[0,99],[100,199],[200,299],[300,399],[400,499]]}
            }}}})
        );
        assert!(members(1, &[]).is_empty());
        let chunks = members(1, &(1..=201).collect::<Vec<_>>());
        assert_eq!(
            chunks
                .iter()
                .map(|c| c.payload["d"]["user_ids"].as_array().unwrap().len())
                .collect::<Vec<_>>(),
            [100, 100, 1]
        );
        assert_eq!(chunks[0].payload["d"]["guild_id"], "1");
        assert_eq!(chunks[0].payload["d"]["user_ids"][0], "1");
        assert_eq!(chunks[0].payload["d"]["presences"], false);
        let mut queue = Commands::new();
        queue.push(false, subscription(1, 2, &ranges));
        assert!(queue.next_send().is_none());
        for command in chunks {
            queue.push(true, command);
        }
        assert!(queue.pop().is_some());
        assert_eq!(
            queue.next_members.duration_since(queue.refilled_at),
            Duration::from_millis(300)
        );
        assert!(queue.next_send().unwrap() > Instant::now() + Duration::from_millis(250));
        for _ in 1..10 {
            queue.record_send(Instant::now());
        }
        assert!(queue.next_send().unwrap() > Instant::now() + Duration::from_millis(500));
        let later = queue.refilled_at + Duration::from_secs(60);
        queue.refill(later);
        assert_eq!(queue.tokens, 10.0);
    }
}
