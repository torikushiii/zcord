use super::packetize::Packet;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const INITIAL_BURST: usize = 8;
const PACKET_INTERVAL: Duration = Duration::from_micros(250);
const STALE_AFTER: Duration = Duration::from_millis(500);

pub(crate) struct Pacer {
    packets: VecDeque<Packet>,
    origin: Option<Instant>,
    sent: usize,
    progress: Instant,
    deadline: Instant,
}

impl Default for Pacer {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            packets: VecDeque::new(),
            origin: None,
            sent: 0,
            progress: now,
            deadline: now,
        }
    }
}

impl Pacer {
    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    pub fn clear(&mut self) {
        self.packets.clear();
        self.origin = None;
        self.sent = 0;
    }

    pub fn stale(&self, now: Instant) -> bool {
        !self.is_empty() && now.duration_since(self.progress) >= STALE_AFTER
    }

    pub fn queue(&mut self, packets: Vec<Packet>, now: Instant) {
        self.packets = packets.into();
        self.origin = Some(now);
        self.sent = 0;
        self.progress = now;
        self.deadline = now;
    }

    pub fn next_batch(&mut self, now: Instant) -> Vec<Packet> {
        let Some(origin) = self.origin else {
            return Vec::new();
        };
        let elapsed = now.saturating_duration_since(origin);
        let allowed = INITIAL_BURST
            .saturating_add((elapsed.as_micros() / PACKET_INTERVAL.as_micros()) as usize);
        let count = allowed.saturating_sub(self.sent).min(self.packets.len());
        let mut batch = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some(packet) = self.packets.pop_front() {
                batch.push(packet);
            }
        }
        self.sent += count;
        if count > 0 {
            self.progress = now;
        }
        self.deadline = if self.packets.is_empty() {
            now
        } else {
            origin
                .checked_add(
                    PACKET_INTERVAL.saturating_mul(self.sent.saturating_sub(INITIAL_BURST) as u32),
                )
                .unwrap_or(now)
                .max(now)
        };
        batch
    }

    pub fn deadline(&self) -> Instant {
        self.deadline
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::voice::stream::packetize::packetize;

    fn packets(count: usize) -> Vec<Packet> {
        let mut sequence = 0;
        let mut transport = 0;
        let mut packets = Vec::new();
        for _ in 0..count {
            packets.extend(
                packetize(&[0, 0, 0, 1, 0x41, 1], &mut sequence, &mut transport, 0, 1).unwrap(),
            );
        }
        packets
    }

    #[test]
    fn first_eight_packets_go_at_once_then_every_quarter_millisecond() {
        let now = Instant::now();
        let mut pacer = Pacer::default();
        pacer.queue(packets(12), now);
        assert_eq!(pacer.next_batch(now).len(), 8);
        assert_eq!(pacer.next_batch(now).len(), 0);
        assert_eq!(pacer.next_batch(now + Duration::from_micros(250)).len(), 1);
        assert_eq!(pacer.next_batch(now + Duration::from_millis(1)).len(), 3);
        assert!(pacer.is_empty());
    }

    #[test]
    fn a_stalled_access_unit_becomes_stale_after_half_a_second() {
        let now = Instant::now();
        let mut pacer = Pacer::default();
        pacer.queue(packets(20), now);
        let _ = pacer.next_batch(now);
        assert!(!pacer.stale(now));
        assert!(pacer.stale(now + Duration::from_millis(500)));
        pacer.clear();
        assert!(!pacer.stale(now + Duration::from_secs(1)));
    }
}
