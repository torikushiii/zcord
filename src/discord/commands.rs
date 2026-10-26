use serde_json::{Value, json};
use std::collections::VecDeque;
use tokio::time::{Duration, Instant};

pub(super) struct Command {
    pub payload: Value,
    pub members: bool,
}

pub(super) fn voice_state(payload: Value) -> Command {
    Command {
        payload,
        members: false,
    }
}

pub(super) fn voice_start_times(guild_id: u64) -> Command {
    Command {
        members: false,
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
        payload: json!({"op":37,"d":{"subscriptions":{
            guild_id.to_string(): {"typing":true,"activities":true,"threads":false,
                "member_updates":false,"channels":{channel_id.to_string():ranges}}
        }}}),
    }
}

pub(super) fn members(guild_id: u64, user_ids: &[u64]) -> Vec<Command> {
    user_ids
        .chunks(100)
        .map(|chunk| Command {
            members: true,
            payload: json!({"op":8,"d":{"guild_id":guild_id.to_string(),
            "user_ids":chunk.iter().map(u64::to_string).collect::<Vec<_>>(),"presences":false}}),
        })
        .collect()
}

pub(super) struct Commands {
    queue: VecDeque<Command>,
    tokens: f64,
    refilled_at: Instant,
    next_members: Instant,
}

impl Commands {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            tokens: 10.0,
            refilled_at: Instant::now(),
            next_members: Instant::now(),
        }
    }

    pub fn push(&mut self, ready: bool, command: Command) {
        if ready {
            self.queue.push_back(command);
        } else {
            log::debug!("Dropping gateway command before READY/RESUMED");
        }
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
        let command = self.queue.front()?;
        // Burst 10 + refill 100/minute leaves 10 of Discord's 120 slots for heartbeats.
        // https://docs.discord.food/gateway/using-gateway#rate-limiting
        let mut next = now + Duration::from_secs_f64((1.0 - self.tokens).max(0.0) * 0.6);
        if command.members {
            next = next.max(self.next_members);
        }
        Some(next)
    }

    pub fn pop(&mut self) -> Option<Value> {
        let command = self.queue.pop_front()?;
        let now = Instant::now();
        if command.members {
            self.next_members = now + Duration::from_millis(300);
        }
        self.record_send(now);
        Some(command.payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(queue.pop().unwrap()["op"], 43);
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
                "typing":true,"activities":true,"threads":false,"member_updates":false,
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
