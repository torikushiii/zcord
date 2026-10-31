use super::packetize::Packet;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const MAX_PACKETS: usize = 2048;
const MAX_REPAIRS: usize = 128;
const MAX_RETRIES: u8 = 2;
const RETRY_DELAY: Duration = Duration::from_millis(50);

struct Sent {
    packet: Packet,
    retries: u8,
    last_retry: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct History {
    packets: VecDeque<Sent>,
    pending: VecDeque<u16>,
}

impl History {
    pub fn clear(&mut self) {
        self.packets.clear();
        self.pending.clear();
    }

    pub fn remember(&mut self, packet: Packet) {
        if self.packets.len() == MAX_PACKETS {
            self.packets.pop_front();
        }
        self.packets.push_back(Sent {
            packet,
            retries: 0,
            last_retry: None,
        });
    }

    /// Returns true when a requested packet is no longer cached and needs a fresh keyframe.
    pub fn request(&mut self, sequences: &[u16], now: Instant) -> bool {
        let mut missing = false;
        for &sequence in sequences.iter().take(MAX_REPAIRS) {
            let Some(sent) = self
                .packets
                .iter()
                .find(|sent| sent.packet.header[2..4] == sequence.to_be_bytes())
            else {
                missing = true;
                continue;
            };
            if sent.retries >= MAX_RETRIES {
                missing = true;
                continue;
            }
            if self.pending.len() < MAX_REPAIRS
                && !self.pending.contains(&sequence)
                && sent
                    .last_retry
                    .is_none_or(|at| now.duration_since(at) >= RETRY_DELAY)
            {
                self.pending.push_back(sequence);
            }
        }
        missing
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn repair(
        &mut self,
        rtx_ssrc: u32,
        rtx_sequence: &mut u16,
        transport_sequence: &mut u16,
        now: Instant,
    ) -> Option<Packet> {
        if rtx_ssrc == 0 {
            return None;
        }
        while let Some(sequence) = self.pending.pop_front() {
            let Some(sent) = self
                .packets
                .iter_mut()
                .find(|sent| sent.packet.header[2..4] == sequence.to_be_bytes())
            else {
                continue;
            };
            if sent.retries >= MAX_RETRIES {
                continue;
            }
            sent.retries += 1;
            sent.last_retry = Some(now);
            let packet = sent.packet.rtx(rtx_ssrc, *rtx_sequence, transport_sequence);
            *rtx_sequence = rtx_sequence.wrapping_add(1);
            return Some(packet);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::voice::stream::packetize::packetize;

    fn packet(sequence: &mut u16, transport: &mut u16) -> Packet {
        packetize(&[0, 0, 0, 1, 0x41, 1, 2, 3], sequence, transport, 90, 11)
            .unwrap()
            .remove(0)
    }

    #[test]
    fn repairs_a_nack_on_the_rtx_ssrc_with_rfc4588_numbering() {
        let mut sequence = 1;
        let mut transport = 10;
        let mut history = History::default();
        let first = packet(&mut sequence, &mut transport);
        history.remember(first.clone());
        let now = Instant::now();
        assert!(!history.request(&[1], now));
        let mut rtx_sequence = 500;
        let mut rtx_transport = 900;
        let resend = history
            .repair(77, &mut rtx_sequence, &mut rtx_transport, now)
            .unwrap();
        assert_eq!(resend.header[1], 0x80 | 102);
        assert_eq!(&resend.header[2..4], &500u16.to_be_bytes());
        assert_eq!(&resend.header[8..12], &77u32.to_be_bytes());
        assert_eq!(&resend.payload[..2], &[0, 1]);
        assert_eq!(&resend.payload[2..], &first.payload[..]);
        assert_eq!(rtx_sequence, 501);
        assert_eq!(rtx_transport, 901);
    }

    #[test]
    fn missing_packets_and_exhausted_retries_ask_for_a_keyframe() {
        let mut history = History::default();
        assert!(history.request(&[9], Instant::now()));
        let mut sequence = 1;
        let mut transport = 10;
        history.remember(packet(&mut sequence, &mut transport));
        let now = Instant::now();
        assert!(!history.request(&[1], now));
        let mut rtx_sequence = 0;
        let mut rtx_transport = 0;
        for _ in 0..2 {
            history
                .repair(7, &mut rtx_sequence, &mut rtx_transport, now)
                .expect("queued repair");
            history.request(&[1], now + Duration::from_millis(60));
        }
        assert!(history.request(&[1], now + Duration::from_millis(120)));
    }

    #[test]
    fn history_and_repairs_are_bounded() {
        let mut history = History::default();
        let mut sequence = 0;
        let mut transport = 0;
        for _ in 0..(MAX_PACKETS + 10) {
            history.remember(packet(&mut sequence, &mut transport));
        }
        assert_eq!(history.packets.len(), MAX_PACKETS);
        let now = Instant::now();
        let all: Vec<u16> = (0..300).collect();
        assert!(history.request(&all, now));
        assert!(history.pending.len() <= MAX_REPAIRS);
    }
}
