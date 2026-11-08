use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub(crate) const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const GAP_TIMEOUT: Duration = Duration::from_millis(300);
const NACK_INTERVAL: Duration = Duration::from_millis(80);
const NACK_ATTEMPTS: u8 = 3;
const MAX_PENDING: usize = 256;
const START_CODE: [u8; 4] = [0, 0, 0, 1];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Packet {
    pub sequence: u16,
    pub timestamp: u32,
    pub marker: bool,
    pub payload: Vec<u8>,
}

impl Packet {
    pub fn new(sequence: u16, timestamp: u32, marker: bool, payload: Vec<u8>) -> Self {
        Self {
            sequence,
            timestamp,
            marker,
            payload,
        }
    }
}

struct Gap {
    first: Instant,
    last: Instant,
    attempts: u8,
}

#[derive(Default)]
pub(crate) struct Reorder {
    expected: Option<u16>,
    pending: Vec<Packet>,
    gaps: HashMap<u16, Gap>,
    skipped: bool,
}

impl Reorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, packet: Packet, now: Instant) -> Vec<Packet> {
        let sequence = packet.sequence;
        let Some(expected) = self.expected else {
            self.expected = Some(sequence.wrapping_add(1));
            return vec![packet];
        };
        let distance = sequence.wrapping_sub(expected);
        if distance >= 32768 {
            return Vec::new();
        }
        if distance == 0 {
            self.expected = Some(sequence.wrapping_add(1));
            self.gaps.remove(&sequence);
            let mut released = vec![packet];
            self.drain(&mut released);
            return released;
        }
        if usize::from(distance) >= MAX_PENDING {
            self.pending.clear();
            self.gaps.clear();
            self.expected = Some(sequence.wrapping_add(1));
            self.skipped = true;
            return vec![packet];
        }
        if self
            .pending
            .iter()
            .any(|packet| packet.sequence == sequence)
        {
            return Vec::new();
        }
        let mut missing = expected;
        while missing != sequence {
            self.gaps.entry(missing).or_insert_with(|| Gap {
                first: now,
                last: now,
                attempts: 0,
            });
            missing = missing.wrapping_add(1);
        }
        self.pending.push(packet);
        Vec::new()
    }

    pub fn missing(&mut self, now: Instant) -> Vec<u16> {
        let mut missing = Vec::new();
        for (&sequence, gap) in &mut self.gaps {
            let due = gap.attempts == 0 || now.duration_since(gap.last) >= NACK_INTERVAL;
            if gap.attempts < NACK_ATTEMPTS && due {
                gap.attempts += 1;
                gap.last = now;
                missing.push(sequence);
            }
        }
        missing.sort_unstable();
        missing
    }

    pub fn expire(&mut self, now: Instant) -> Vec<Packet> {
        let mut skipped = false;
        loop {
            let expected = self.expected;
            let Some(&sequence) = self
                .gaps
                .iter()
                .filter(|(_, gap)| now.duration_since(gap.first) >= GAP_TIMEOUT)
                .filter(|(sequence, _)| {
                    expected.is_none_or(|expected| sequence.wrapping_sub(expected) < 32768)
                })
                .min_by_key(|(sequence, _)| {
                    expected.map_or(0, |expected| sequence.wrapping_sub(expected))
                })
                .map(|(sequence, _)| sequence)
            else {
                break;
            };
            self.gaps.remove(&sequence);
            self.expected = Some(sequence.wrapping_add(1));
            skipped = true;
        }
        self.pending.retain(|packet| {
            self.expected
                .is_none_or(|expected| packet.sequence.wrapping_sub(expected) < 32768)
        });
        let mut released = Vec::new();
        self.drain(&mut released);
        self.skipped |= skipped;
        released
    }

    pub fn take_skipped(&mut self) -> bool {
        std::mem::take(&mut self.skipped)
    }

    fn drain(&mut self, released: &mut Vec<Packet>) {
        loop {
            let Some(expected) = self.expected else {
                return;
            };
            let Some(index) = self
                .pending
                .iter()
                .position(|packet| packet.sequence == expected)
            else {
                return;
            };
            released.push(self.pending.swap_remove(index));
            self.gaps.remove(&expected);
            self.expected = Some(expected.wrapping_add(1));
        }
    }
}

#[derive(Default)]
pub(crate) struct Assembler {
    timestamp: u32,
    next_sequence: Option<u16>,
    frame: Vec<u8>,
    fragmenting: bool,
    broken: bool,
    started: bool,
    lost: bool,
}

impl Assembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, packet: &Packet) -> Option<Vec<u8>> {
        if self.started && packet.timestamp != self.timestamp {
            self.lost = true;
            self.reset_frame();
        }
        if !self.started {
            self.started = true;
            self.timestamp = packet.timestamp;
        }
        if self
            .next_sequence
            .is_some_and(|expected| expected != packet.sequence)
        {
            self.broken = true;
            self.lost = true;
        }
        self.next_sequence = Some(packet.sequence.wrapping_add(1));
        if !self.broken && self.append(&packet.payload).is_err() {
            self.broken = true;
        }
        self.lost |= self.broken;
        if !packet.marker {
            return None;
        }
        let complete = (!self.broken && !self.fragmenting && !self.frame.is_empty())
            .then(|| std::mem::take(&mut self.frame));
        self.lost |= complete.is_none();
        self.reset_frame();
        self.started = false;
        complete
    }

    pub fn mark_lost(&mut self) {
        self.broken = true;
        self.lost = true;
    }

    pub fn take_lost(&mut self) -> bool {
        std::mem::take(&mut self.lost)
    }

    fn reset_frame(&mut self) {
        self.frame.clear();
        self.fragmenting = false;
        self.broken = false;
    }

    fn extend(&mut self, parts: &[&[u8]]) -> Result<(), ()> {
        let total: usize = parts.iter().map(|part| part.len()).sum();
        if self.frame.len().saturating_add(total) > MAX_FRAME_BYTES {
            return Err(());
        }
        for part in parts {
            self.frame.extend_from_slice(part);
        }
        Ok(())
    }

    fn append(&mut self, payload: &[u8]) -> Result<(), ()> {
        let Some(&indicator) = payload.first() else {
            return Err(());
        };
        match indicator & 0x1f {
            1..=23 => {
                if self.fragmenting {
                    return Err(());
                }
                self.extend(&[&START_CODE, payload])
            }
            24 => {
                if self.fragmenting {
                    return Err(());
                }
                let mut at = 1;
                while at < payload.len() {
                    if at + 2 > payload.len() {
                        return Err(());
                    }
                    let size = usize::from(u16::from_be_bytes([payload[at], payload[at + 1]]));
                    at += 2;
                    if size == 0 || at + size > payload.len() {
                        return Err(());
                    }
                    self.extend(&[&START_CODE, &payload[at..at + size]])?;
                    at += size;
                }
                Ok(())
            }
            28 => {
                if payload.len() < 2 {
                    return Err(());
                }
                let header = payload[1];
                let start = header & 0x80 != 0;
                let end = header & 0x40 != 0;
                if start == self.fragmenting {
                    return Err(());
                }
                if start {
                    let nal_header = (indicator & 0xe0) | (header & 0x1f);
                    self.extend(&[&START_CODE, &[nal_header], &payload[2..]])?;
                } else {
                    self.extend(&[&payload[2..]])?;
                }
                self.fragmenting = !end;
                Ok(())
            }
            _ => Err(()),
        }
    }
}

pub(crate) fn is_keyframe(frame: &[u8]) -> bool {
    nal_units(frame).any(|nal| nal.first().is_some_and(|byte| byte & 0x1f == 5))
}

pub(crate) fn has_parameter_sets(frame: &[u8]) -> bool {
    let (mut sps, mut pps) = (false, false);
    for nal in nal_units(frame) {
        match nal.first().map(|byte| byte & 0x1f) {
            Some(7) => sps = true,
            Some(8) => pps = true,
            _ => {}
        }
    }
    sps && pps
}

fn nal_units(frame: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut at = 0;
    std::iter::from_fn(move || {
        while at + 3 <= frame.len() {
            let size = if frame[at..].starts_with(&[0, 0, 0, 1]) {
                4
            } else if frame[at..].starts_with(&[0, 0, 1]) {
                3
            } else {
                at += 1;
                continue;
            };
            let start = at + size;
            let mut end = start;
            while end + 3 <= frame.len()
                && !frame[end..].starts_with(&[0, 0, 1])
                && !frame[end..].starts_with(&[0, 0, 0, 1])
            {
                end += 1;
            }
            if end + 3 > frame.len() {
                end = frame.len();
            }
            at = end;
            if start < end {
                return Some(&frame[start..end]);
            }
        }
        None
    })
}

pub(crate) fn depacketize_rtx(payload: &[u8]) -> (u16, &[u8]) {
    if payload.len() < 2 {
        return (0, &[]);
    }
    (u16::from_be_bytes([payload[0], payload[1]]), &payload[2..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(sequence: u16, timestamp: u32, marker: bool, payload: &[u8]) -> Packet {
        Packet::new(sequence, timestamp, marker, payload.to_vec())
    }

    #[test]
    fn reorder_releases_in_order_and_wraps() {
        let now = Instant::now();
        let mut reorder = Reorder::new();
        assert_eq!(reorder.push(packet(65534, 1, false, &[1]), now).len(), 1);
        assert_eq!(reorder.push(packet(65535, 1, false, &[2]), now).len(), 1);
        assert_eq!(reorder.push(packet(0, 1, false, &[3]), now).len(), 1);
        assert_eq!(reorder.push(packet(1, 1, true, &[4]), now).len(), 1);
        assert!(reorder.missing(now).is_empty());
    }

    #[test]
    fn reorder_holds_gaps_and_reports_them_three_times() {
        let start = Instant::now();
        let mut reorder = Reorder::new();
        assert_eq!(reorder.push(packet(1, 1, false, &[1]), start).len(), 1);
        assert!(reorder.push(packet(3, 1, true, &[3]), start).is_empty());
        assert_eq!(reorder.missing(start), [2]);
        assert!(reorder.missing(start).is_empty());
        assert_eq!(reorder.missing(start + Duration::from_millis(80)), [2]);
        assert_eq!(reorder.missing(start + Duration::from_millis(160)), [2]);
        assert!(
            reorder
                .missing(start + Duration::from_millis(240))
                .is_empty()
        );
        let released = reorder.push(packet(2, 1, false, &[2]), start);
        assert_eq!(
            released
                .iter()
                .map(|packet| packet.sequence)
                .collect::<Vec<_>>(),
            [2, 3]
        );
        assert!(!reorder.take_skipped());
    }

    #[test]
    fn reorder_gives_up_after_three_hundred_milliseconds() {
        let start = Instant::now();
        let mut reorder = Reorder::new();
        reorder.push(packet(10, 1, false, &[1]), start);
        assert!(reorder.push(packet(12, 1, true, &[3]), start).is_empty());
        assert!(
            reorder
                .expire(start + Duration::from_millis(299))
                .is_empty()
        );
        assert!(!reorder.take_skipped());
        let released = reorder.expire(start + Duration::from_millis(300));
        assert_eq!(
            released
                .iter()
                .map(|packet| packet.sequence)
                .collect::<Vec<_>>(),
            [12]
        );
        assert!(reorder.take_skipped());
    }

    #[test]
    fn assembler_handles_single_stap_a_and_fu_a() {
        let mut assembler = Assembler::new();
        let single = assembler.push(&packet(1, 7, true, &[0x65, 0xAA])).unwrap();
        assert_eq!(single, [0, 0, 0, 1, 0x65, 0xAA]);
        assert!(is_keyframe(&single));

        let stap = [0x78, 0x00, 0x02, 0x67, 0x01, 0x00, 0x02, 0x68, 0x02];
        let stap = assembler.push(&packet(2, 8, true, &stap)).unwrap();
        assert_eq!(stap, [0, 0, 0, 1, 0x67, 0x01, 0, 0, 0, 1, 0x68, 0x02]);
        assert!(has_parameter_sets(&stap));

        assert!(
            assembler
                .push(&packet(3, 9, false, &[0x7c, 0x85, 0x11]))
                .is_none()
        );
        let fu = assembler
            .push(&packet(4, 9, true, &[0x7c, 0x45, 0x22]))
            .unwrap();
        assert_eq!(fu, [0, 0, 0, 1, 0x65, 0x11, 0x22]);
        assert!(is_keyframe(&fu));
    }

    #[test]
    fn assembler_drops_a_frame_with_a_lost_fragment() {
        let mut assembler = Assembler::new();
        assert!(
            assembler
                .push(&packet(1, 1, false, &[0x7c, 0x85, 0x11]))
                .is_none()
        );
        assert!(
            assembler
                .push(&packet(3, 1, true, &[0x7c, 0x45, 0x22]))
                .is_none()
        );
        assert!(assembler.take_lost());
    }

    #[test]
    fn assembler_bounds_frames_at_two_mebibytes() {
        let mut assembler = Assembler::new();
        let payload = vec![0x65; MAX_FRAME_BYTES];
        assert!(assembler.push(&packet(1, 1, true, &payload)).is_none());
        assert!(assembler.take_lost());
    }

    #[test]
    fn rtx_depacketizes_the_original_sequence_number() {
        assert_eq!(
            depacketize_rtx(&[0x12, 0x34, 0x65, 0xAA]),
            (0x1234, &[0x65, 0xAA][..])
        );
        assert_eq!(depacketize_rtx(&[0x12]), (0, &[][..]));
        assert_eq!(depacketize_rtx(&[0x12, 0x34]), (0x1234, &[][..]));
    }
}
