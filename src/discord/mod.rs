mod gateway;
pub mod model;
mod permissions;
mod properties;
mod rest;

use futures::channel::mpsc::{self, UnboundedReceiver};
use std::{fmt, sync::Arc, time::Duration};
use tokio::sync::watch;

pub(crate) use permissions::can_view;

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
        let session = Self {
            inner: Arc::new(Inner { rest, control }),
        };
        let owner = Arc::downgrade(&session.inner);
        runtime.spawn(gateway::run(token, sender, shutdown, owner));
        (session, receiver)
    }

    pub async fn fetch_messages(
        &self,
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
            result = rest.fetch_messages(channel_id, before) => result,
        };
        if matches!(result, Err(Error::Http(401))) {
            self.inner.control.send_replace(Control::InvalidToken);
        }
        result
    }

    pub fn shutdown(&self) {
        self.inner.control.send_replace(Control::Shutdown);
    }
}

pub enum Event {
    Ready(Box<model::Ready>),
    MessageCreate(Box<model::Message>),
    MessageUpdate(Box<model::MessageUpdate>),
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
