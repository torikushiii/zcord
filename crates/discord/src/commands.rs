use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use tokio::time::{Duration, Instant};

const SUBSCRIBED_CHANNELS: usize = 5;
const MEMBER_SEARCH_REPEAT: Duration = Duration::from_secs(60);

pub(super) struct Command {
    pub payload: Value,
    pub kind: Kind,
    pub members: bool,
    pub presence_revision: Option<u64>,
    pub completion: Option<tokio::sync::oneshot::Sender<Result<(), super::Error>>>,
}

pub(super) enum Kind {
    Send,
    Subscribe {
        guild_id: u64,
        channel_id: u64,
        ranges: Vec<[u32; 2]>,
    },
    ChannelInfo(u64),
    CallConnect(u64),
    MemberSearch(u64, String),
    Activity,
}

impl Command {
    fn new(payload: Value, kind: Kind) -> Self {
        Self {
            payload,
            kind,
            members: false,
            presence_revision: None,
            completion: None,
        }
    }

    pub fn complete(self, result: Result<(), super::Error>) {
        if let Some(completion) = self.completion {
            let _ = completion.send(result);
        }
    }
}

pub(super) fn presence(payload: Value, revision: u64) -> Command {
    Command {
        presence_revision: Some(revision),
        ..Command::new(payload, Kind::Send)
    }
}

pub(super) fn voice_state(payload: Value) -> Command {
    Command::new(payload, Kind::Send)
}

pub(super) fn activity() -> Command {
    Command::new(Value::Null, Kind::Activity)
}

pub(super) fn call_connect(channel_id: u64) -> Command {
    Command::new(
        json!({"op":13,"d":{"channel_id":channel_id.to_string()}}),
        Kind::CallConnect(channel_id),
    )
}

pub(super) fn voice_start_times(guild_id: u64) -> Command {
    Command::new(
        json!({"op":43,"d":{
            "guild_id":guild_id.to_string(),"fields":["status","voice_start_time"]
        }}),
        Kind::ChannelInfo(guild_id),
    )
}

pub(super) fn subscription(guild_id: u64, channel_id: u64, ranges: &[(u32, u32)]) -> Command {
    let ranges = ranges
        .iter()
        .take(5)
        .filter(|(start, end)| start <= end)
        .map(|&(start, end)| [start, end])
        .collect();
    Command::new(
        Value::Null,
        Kind::Subscribe {
            guild_id,
            channel_id,
            ranges,
        },
    )
}

pub(super) fn members(guild_id: u64, user_ids: &[u64]) -> Vec<Command> {
    user_ids
        .chunks(100)
        .map(|chunk| Command {
            members: true,
            ..Command::new(
                json!({"op":8,"d":{
                    "guild_id":[guild_id.to_string()],"query":null,"limit":null,
                    "user_ids":chunk.iter().map(u64::to_string).collect::<Vec<_>>(),
                    "presences":false,"nonce":null
                }}),
                Kind::Send,
            )
        })
        .collect()
}

pub(super) fn query_members(guild_id: u64, query: &str, limit: u32) -> Command {
    let query = query.to_lowercase();
    Command {
        members: true,
        ..Command::new(
            json!({"op":8,"d":{
                "guild_id":[guild_id.to_string()],"query":query,"limit":limit,
                "user_ids":null,"presences":true,"nonce":null
            }}),
            Kind::MemberSearch(guild_id, query),
        )
    }
}

#[derive(Default)]
pub(super) struct Requests {
    guilds: Vec<GuildSubscription>,
    latest_guild: Option<u64>,
    channel_info: HashSet<u64>,
    calls: HashSet<u64>,
    searches: HashMap<(u64, String), Instant>,
}

struct GuildSubscription {
    guild_id: u64,
    channels: VecDeque<(u64, Vec<[u32; 2]>)>,
}

impl GuildSubscription {
    fn channels(&self) -> Map<String, Value> {
        self.channels
            .iter()
            .map(|(channel_id, ranges)| (channel_id.to_string(), json!(ranges)))
            .collect()
    }

    fn full(&self) -> Value {
        json!({
            "typing":true,"threads":true,"activities":true,"members":[],"member_updates":false,
            "channels":self.channels(),"thread_member_lists":[]
        })
    }
}

impl Requests {
    pub fn admit(&mut self, mut command: Command, now: Instant) -> Option<Command> {
        match &command.kind {
            Kind::Send | Kind::Activity => {}
            Kind::Subscribe {
                guild_id,
                channel_id,
                ranges,
            } => {
                let delta = self.subscribe(*guild_id, *channel_id, ranges)?;
                command.payload = json!({"op":37,"d":{"subscriptions":{
                    guild_id.to_string(): delta
                }}});
            }
            Kind::ChannelInfo(guild_id) => {
                if !self.channel_info.insert(*guild_id) {
                    return None;
                }
            }
            Kind::CallConnect(channel_id) => {
                if !self.calls.insert(*channel_id) {
                    return None;
                }
            }
            Kind::MemberSearch(guild_id, query) => {
                self.searches
                    .retain(|_, sent| now.duration_since(*sent) < MEMBER_SEARCH_REPEAT);
                if self.searches.contains_key(&(*guild_id, query.clone())) {
                    return None;
                }
                self.searches.insert((*guild_id, query.clone()), now);
            }
        }
        Some(command)
    }

    fn subscribe(&mut self, guild_id: u64, channel_id: u64, ranges: &[[u32; 2]]) -> Option<Value> {
        self.latest_guild = Some(guild_id);
        let mut delta = Map::new();
        let index = match self
            .guilds
            .iter()
            .position(|guild| guild.guild_id == guild_id)
        {
            Some(index) => index,
            None => {
                for feature in ["typing", "activities", "threads"] {
                    delta.insert(feature.into(), true.into());
                }
                self.guilds.push(GuildSubscription {
                    guild_id,
                    channels: VecDeque::new(),
                });
                self.guilds.len() - 1
            }
        };
        let guild = &mut self.guilds[index];
        let previous = guild
            .channels
            .iter()
            .position(|(id, _)| *id == channel_id)
            .and_then(|index| guild.channels.remove(index));
        let changed = previous.is_none_or(|(_, previous)| previous != ranges);
        guild.channels.push_front((channel_id, ranges.to_vec()));
        guild.channels.truncate(SUBSCRIBED_CHANNELS);
        if changed {
            delta.insert("channels".into(), guild.channels().into());
        }
        (!delta.is_empty()).then_some(Value::Object(delta))
    }

    fn resync(&self) -> Option<Command> {
        let subscriptions: Map<String, Value> = self
            .guilds
            .iter()
            .map(|guild| (guild.guild_id.to_string(), guild.full()))
            .collect();
        (!subscriptions.is_empty()).then(|| {
            Command::new(
                json!({"op":37,"d":{"subscriptions":subscriptions}}),
                Kind::Send,
            )
        })
    }

    pub fn ready(&mut self) -> Option<Command> {
        let latest = self.latest_guild;
        self.guilds.retain(|guild| Some(guild.guild_id) == latest);
        self.channel_info.clear();
        self.calls.clear();
        self.searches.clear();
        self.resync()
    }

    pub fn resumed(&mut self) -> Option<Command> {
        self.channel_info.clear();
        self.calls.clear();
        self.resync()
    }

    pub fn guild_changed(&mut self, guild_id: u64) {
        self.channel_info.remove(&guild_id);
    }

    pub fn call_seen(&mut self, channel_id: u64) {
        self.calls.insert(channel_id);
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
                "guild_id":["123"],"query":"carol m","limit":10,
                "user_ids":null,"presences":true,"nonce":null
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
        assert_eq!(queue.pop().unwrap().payload["d"]["query"], "carol m");
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
        let subscribe = Requests::default().admit(subscription(1, 2, &[]), Instant::now());
        queue.push(true, subscribe.unwrap());
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
            json!({"op":43,"d":{"guild_id":"123","fields":["status","voice_start_time"]}})
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
        let payload = Requests::default()
            .admit(subscription(1, 2, &ranges), Instant::now())
            .unwrap()
            .payload;
        assert_eq!(
            payload,
            json!({"op":37,"d":{"subscriptions":{"1":{
                "typing":true,"activities":true,"threads":true,
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
        assert_eq!(chunks[0].payload["d"]["guild_id"], json!(["1"]));
        assert!(chunks[0].payload["d"]["query"].is_null());
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

    fn admitted(requests: &mut Requests, command: Command) -> Option<Value> {
        requests
            .admit(command, Instant::now())
            .map(|command| command.payload["d"]["subscriptions"].clone())
    }

    #[test]
    fn subscriptions_send_deltas_and_keep_five_recent_channels_per_guild() {
        let mut requests = Requests::default();
        assert_eq!(
            admitted(&mut requests, subscription(1, 10, &[(0, 99)])),
            Some(json!({"1":{
                "typing":true,"activities":true,"threads":true,"channels":{"10":[[0,99]]}
            }}))
        );
        assert_eq!(
            admitted(&mut requests, subscription(1, 10, &[(0, 99)])),
            None
        );
        assert_eq!(
            admitted(&mut requests, subscription(1, 10, &[(0, 99), (100, 199)])),
            Some(json!({"1":{"channels":{"10":[[0,99],[100,199]]}}}))
        );
        for channel_id in 11..=15 {
            admitted(&mut requests, subscription(1, channel_id, &[(0, 99)]));
        }
        let channels =
            admitted(&mut requests, subscription(1, 11, &[(0, 199)])).unwrap()["1"]["channels"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>();
        let mut expected = ["11", "15", "14", "13", "12"];
        let mut sorted = channels.clone();
        sorted.sort();
        expected.sort();
        assert_eq!(sorted, expected);
        assert_eq!(
            admitted(&mut requests, subscription(2, 20, &[])),
            Some(json!({"2":{
                "typing":true,"activities":true,"threads":true,"channels":{"20":[]}
            }}))
        );
    }

    #[test]
    fn ready_keeps_the_latest_guild_and_resumed_resends_every_guild() {
        let mut requests = Requests::default();
        assert!(requests.ready().is_none());
        admitted(&mut requests, subscription(1, 10, &[(0, 99)]));
        admitted(&mut requests, subscription(2, 20, &[(0, 99)]));
        let full = |channel: &str| {
            json!({
                "typing":true,"threads":true,"activities":true,"members":[],
                "member_updates":false,"channels":{channel:[[0,99]]},"thread_member_lists":[]
            })
        };
        let resumed = requests.resumed().unwrap().payload;
        assert_eq!(
            resumed["d"]["subscriptions"],
            json!({"1":full("10"),"2":full("20")})
        );
        let ready = requests.ready().unwrap().payload;
        assert_eq!(ready["op"], 37);
        assert_eq!(ready["d"]["subscriptions"], json!({"2":full("20")}));
        assert_eq!(
            admitted(&mut requests, subscription(2, 20, &[(0, 99)])),
            None
        );
        assert!(
            admitted(&mut requests, subscription(1, 10, &[(0, 99)])).unwrap()["1"]["typing"]
                .as_bool()
                .unwrap()
        );
    }

    #[test]
    fn channel_info_calls_and_searches_are_sent_once_per_connection() {
        let mut requests = Requests::default();
        let now = Instant::now();
        assert!(requests.admit(voice_start_times(1), now).is_some());
        assert!(requests.admit(voice_start_times(1), now).is_none());
        requests.guild_changed(1);
        assert!(requests.admit(voice_start_times(1), now).is_some());
        assert!(requests.resumed().is_none());
        assert!(requests.admit(voice_start_times(1), now).is_some());

        assert_eq!(
            call_connect(5).payload,
            json!({"op":13,"d":{"channel_id":"5"}})
        );
        assert!(requests.admit(call_connect(5), now).is_some());
        assert!(requests.admit(call_connect(5), now).is_none());
        requests.call_seen(6);
        assert!(requests.admit(call_connect(6), now).is_none());
        requests.resumed();
        assert!(requests.admit(call_connect(5), now).is_some());

        assert!(requests.admit(query_members(1, "Ab", 10), now).is_some());
        assert!(requests.admit(query_members(1, "aB", 10), now).is_none());
        assert!(requests.admit(query_members(2, "ab", 10), now).is_some());
        let later = now + MEMBER_SEARCH_REPEAT;
        assert!(requests.admit(query_members(1, "ab", 10), later).is_some());

        requests.ready();
        assert!(requests.admit(voice_start_times(1), now).is_some());
        assert!(requests.admit(call_connect(5), now).is_some());
        assert!(requests.admit(query_members(2, "ab", 10), now).is_some());
    }
}
