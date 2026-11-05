mod audio;
mod crypto;
mod dave;
mod gateway;
mod rtcp;
mod rtp;
mod stream;
mod task;
mod udp;

use crate::{Event, VoiceEvent};
pub use audio::{AudioDevice, AudioDeviceKind, AudioDevices, MicTest, audio_devices};
use futures::channel::mpsc::UnboundedSender;
pub(crate) use stream::stream_key;
pub use stream::{FrameSlot, ScreenSource, StreamFrame, StreamQuality};

pub(super) use task::VoiceManager;

pub(super) fn emit(events: &UnboundedSender<Event>, event: VoiceEvent) {
    let _ = events.unbounded_send(Event::Voice(event));
}
