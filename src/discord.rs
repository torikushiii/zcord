mod commands;
mod gateway;
pub mod model;
mod permissions;
mod properties;
mod rest;
#[cfg(test)]
pub mod settings_fixtures;
pub mod settings_proto;

use futures::channel::mpsc::{self, UnboundedReceiver};
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;

pub use permissions::can_view;
pub use properties::USER_AGENT;

struct Secret(String);
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Running,
    Shutdown,
    InvalidToken,
}

struct Inner {
    rest: Result<rest::Rest, Error>,
    control: watch::Sender<Control>,
    commands: tokio::sync::mpsc::UnboundedSender<(u64, commands::Command)>,
    generation: Arc<AtomicU64>,
}

#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

impl Session {
    pub fn connect(
        token: String,
        runtime: &tokio::runtime::Handle,
    ) -> (Session, UnboundedReceiver<Event>) {
        let token = Arc::new(Secret(token));
        let (sender, receiver) = mpsc::unbounded();
        let (control, shutdown) = watch::channel(Control::Running);
        let rest = rest::Rest::new(&token.0);
        let (commands, command_receiver) = tokio::sync::mpsc::unbounded_channel();
        let generation = Arc::new(AtomicU64::new(0));
        let session = Self {
            inner: Arc::new(Inner {
                rest,
                control,
                commands,
                generation: generation.clone(),
            }),
        };
        let owner = Arc::downgrade(&session.inner);
        runtime.spawn(gateway::run(
            token,
            sender,
            shutdown,
            owner,
            command_receiver,
            generation,
        ));
        (session, receiver)
    }

    pub async fn fetch_messages(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        before: Option<u64>,
    ) -> Result<Vec<model::Message>, Error> {
        let mut shutdown = self.inner.control.subscribe();
        if *shutdown.borrow() != Control::Running {
            return Err(Error::Closed);
        }
        let rest = self.inner.rest.as_ref().map_err(Clone::clone)?;
        let result = tokio::select! {
            biased;
            _ = shutdown.changed() => Err(Error::Closed),
            result = rest.fetch_messages(guild_id, channel_id, before) => result,
        };
        if matches!(result, Err(Error::Http(401))) {
            self.inner.control.send_replace(Control::InvalidToken);
        }
        result
    }

    pub fn shutdown(&self) {
        self.inner.control.send_replace(Control::Shutdown);
    }

    fn command(&self, command: commands::Command) {
        let generation = self.inner.generation.load(Ordering::Acquire);
        if generation % 2 == 0 || *self.inner.control.borrow() != Control::Running {
            log::debug!("Dropping gateway command while disconnected/connecting");
            return;
        }
        let _ = self.inner.commands.send((generation, command));
    }

    pub fn subscribe_channel(&self, guild_id: u64, channel_id: u64, ranges: &[(u32, u32)]) {
        self.command(commands::subscription(guild_id, channel_id, ranges));
    }

    pub fn request_members(&self, guild_id: u64, user_ids: Vec<u64>) {
        for command in commands::members(guild_id, &user_ids) {
            self.command(command);
        }
    }
}

pub enum Event {
    ReadySupplemental(Box<model::ReadySupplemental>),
    PresenceUpdate(Box<model::Presence>),
    MembersChunk(Box<model::MembersChunk>),
    MemberListUpdate(Box<model::MemberListUpdate>),
    MemberUpsert(Box<model::MemberUpdate>),
    MemberRemove(Box<model::MemberUpdate>),
    RoleUpsert(Box<model::RoleUpdate>),
    RoleDelete {
        guild_id: u64,
        role_id: u64,
    },
    Ready(Box<model::Ready>),
    SettingsUpdate(Box<model::SettingsUpdate>),
    MessageCreate(Box<model::Message>),
    MessageUpdate(Box<model::MessageUpdate>),
    MessageReactionAdd(model::ReactionEvent),
    MessageReactionRemove(model::ReactionEvent),
    MessageReactionRemoveAll(model::ReactionEvent),
    MessageReactionRemoveEmoji(model::ReactionEvent),
    MessageReactionAddMany(model::ReactionManyEvent),
    MessageDelete {
        channel_id: u64,
        id: u64,
    },
    ChannelUpsert(Box<model::Channel>),
    ChannelDelete {
        id: u64,
    },
    GuildUpsert(Box<model::Guild>),
    GuildDelete {
        id: u64,
    },
    /// Received from another client only. We never acknowledge messages ourselves.
    MessageAck {
        channel_id: u64,
        message_id: u64,
    },
    Reconnecting,
    Resumed,
    Fatal(FatalError),
}

#[derive(Debug)]
pub enum FatalError {
    InvalidToken,
    Other(String),
}

#[derive(Clone, Debug)]
pub enum Error {
    RateLimited { retry_after: Duration },
    Http(u16),
    Network,
    Decode,
    InvalidCredential,
    Closed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RateLimited { retry_after } => write!(
                f,
                "Rate limited; retry after {:.1}s",
                retry_after.as_secs_f64()
            ),
            Self::Http(status) => write!(f, "HTTP {status}"),
            Self::Network => f.write_str("Network request failed"),
            Self::Decode => f.write_str("Malformed Discord response"),
            Self::InvalidCredential => f.write_str("Invalid credential header"),
            Self::Closed => f.write_str("Session closed"),
        }
    }
}
impl std::error::Error for Error {}
