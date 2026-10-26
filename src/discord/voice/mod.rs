mod audio;
mod crypto;
mod dave;
mod gateway;
mod rtcp;
mod rtp;
mod task;
mod udp;

use crate::discord::{Event, VoiceEvent};
pub use audio::{AudioDevice, MicTest, audio_devices};
use futures::channel::mpsc::UnboundedSender;

pub(super) use task::VoiceManager;

pub(super) fn emit(events: &UnboundedSender<Event>, event: VoiceEvent) {
    let _ = events.unbounded_send(Event::Voice(event));
}
