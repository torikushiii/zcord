mod audio;
mod crypto;
mod dave;
mod gateway;
mod rtcp;
mod rtp;
mod task;
mod udp;

use crate::discord::{Event, VoiceEvent};
use futures::channel::mpsc::UnboundedSender;

pub(super) use task::VoiceManager;

pub(super) fn emit(events: &UnboundedSender<Event>, event: VoiceEvent) {
    let _ = events.unbounded_send(Event::Voice(event));
}
