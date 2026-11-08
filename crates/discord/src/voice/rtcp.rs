pub(super) fn packet_type(packet: &[u8]) -> Option<u8> {
    let packet_type = *packet.get(1)?;
    (192..=223).contains(&packet_type).then_some(packet_type)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ReportBlock {
    pub ssrc: u32,
    pub fraction_lost: u8,
    pub cumulative_lost: i32,
}

impl ReportBlock {
    pub(super) fn loss_percent(self) -> f32 {
        f32::from(self.fraction_lost) * 100.0 / 256.0
    }
}

pub(super) fn report_block_for(packet: &[u8], local_ssrc: u32) -> Option<ReportBlock> {
    const SENDER_REPORT: u8 = 200;
    const RECEIVER_REPORT: u8 = 201;
    const BLOCK_LEN: usize = 24;

    let mut offset = 0;
    while let Some(header) = packet.get(offset..offset + 4) {
        if header[0] >> 6 != 2 {
            break;
        }
        let length = (usize::from(u16::from_be_bytes([header[2], header[3]])) + 1) * 4;
        let Some(end) = offset.checked_add(length) else {
            break;
        };
        let Some(body) = packet.get(offset..end) else {
            break;
        };
        let report_count = usize::from(header[0] & 0x1F);
        let blocks_at = match header[1] {
            SENDER_REPORT => 28,
            RECEIVER_REPORT => 8,
            _ => {
                offset = end;
                continue;
            }
        };
        for index in 0..report_count {
            let start = blocks_at + index * BLOCK_LEN;
            let Some(block) = body.get(start..start + BLOCK_LEN) else {
                break;
            };
            let ssrc = u32::from_be_bytes([block[0], block[1], block[2], block[3]]);
            if ssrc == local_ssrc {
                let cumulative = i32::from_be_bytes([0, block[5], block[6], block[7]]);
                return Some(ReportBlock {
                    ssrc,
                    fraction_lost: block[4],
                    cumulative_lost: (cumulative << 8) >> 8,
                });
            }
        }
        offset = end;
    }
    None
}

pub(super) fn generic_nack(sender: u32, media: u32, sequences: &[u16]) -> Vec<u8> {
    let mut sequences = sequences.to_vec();
    sequences.sort_unstable();
    sequences.dedup();
    let mut fcis = Vec::new();
    let mut index = 0;
    while index < sequences.len() {
        let first = sequences[index];
        let mut mask = 0u16;
        index += 1;
        while index < sequences.len() {
            let distance = sequences[index].wrapping_sub(first);
            if distance == 0 || distance > 16 {
                break;
            }
            mask |= 1 << (distance - 1);
            index += 1;
        }
        fcis.push((first, mask));
    }
    let length = u16::try_from(2 + fcis.len()).unwrap_or(u16::MAX);
    let mut packet = Vec::with_capacity(12 + fcis.len() * 4);
    packet.extend_from_slice(&[0x81, 205]);
    packet.extend_from_slice(&length.to_be_bytes());
    packet.extend_from_slice(&sender.to_be_bytes());
    packet.extend_from_slice(&media.to_be_bytes());
    for (pid, mask) in fcis {
        packet.extend_from_slice(&pid.to_be_bytes());
        packet.extend_from_slice(&mask.to_be_bytes());
    }
    packet
}

pub(super) fn pli(sender: u32, media: u32) -> Vec<u8> {
    let mut packet = vec![0x81, 206, 0, 2];
    packet.extend_from_slice(&sender.to_be_bytes());
    packet.extend_from_slice(&media.to_be_bytes());
    packet
}

pub(super) struct ReceiverReport {
    pub media_ssrc: u32,
    pub fraction_lost: u8,
    pub cumulative_lost: i32,
    pub highest_sequence: u32,
    pub jitter: u32,
    pub lsr: u32,
    pub dlsr: u32,
}

pub(super) fn receiver_report(sender: u32, report: &ReceiverReport) -> Vec<u8> {
    let mut packet = vec![0x81, 201, 0, 7];
    packet.extend_from_slice(&sender.to_be_bytes());
    packet.extend_from_slice(&report.media_ssrc.to_be_bytes());
    packet.push(report.fraction_lost);
    let cumulative = report.cumulative_lost.clamp(-(1 << 23), (1 << 23) - 1);
    packet.extend_from_slice(&cumulative.to_be_bytes()[1..]);
    packet.extend_from_slice(&report.highest_sequence.to_be_bytes());
    packet.extend_from_slice(&report.jitter.to_be_bytes());
    packet.extend_from_slice(&report.lsr.to_be_bytes());
    packet.extend_from_slice(&report.dlsr.to_be_bytes());
    packet
}

pub(super) fn sender_report_lsr(packet: &[u8], ssrc: u32) -> Option<u32> {
    let mut offset = 0;
    while let Some(header) = packet.get(offset..offset + 4) {
        if header[0] >> 6 != 2 {
            return None;
        }
        let length = (usize::from(u16::from_be_bytes([header[2], header[3]])) + 1) * 4;
        let end = offset.checked_add(length)?;
        let body = packet.get(offset..end)?;
        if header[1] == 200 && body.len() >= 28 {
            let sender = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
            if sender == ssrc {
                return Some(u32::from_be_bytes([body[10], body[11], body[12], body[13]]));
            }
        }
        offset = end;
    }
    None
}

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct Feedback {
    pub keyframe: bool,
    pub nacks: Vec<u16>,
    pub bitrate: Option<u32>,
    pub loss: Option<u8>,
}

pub(super) fn feedback(packet: &[u8], media_ssrc: u32) -> Option<Feedback> {
    if media_ssrc == 0 {
        return None;
    }
    let mut feedback = Feedback::default();
    let mut offset = 0;
    while let Some(header) = packet.get(offset..offset + 4) {
        if header[0] >> 6 != 2 {
            return None;
        }
        let length = (usize::from(u16::from_be_bytes([header[2], header[3]])) + 1) * 4;
        let end = offset.checked_add(length)?;
        let body = packet.get(offset..end)?;
        let format = header[0] & 0x1f;
        match (header[1], format) {
            (205, 1) => {
                if body.len() < 12 {
                    return None;
                }
                if u32::from_be_bytes([body[8], body[9], body[10], body[11]]) != media_ssrc {
                    offset = end;
                    continue;
                }
                for fci in body[12..].chunks_exact(4) {
                    let pid = u16::from_be_bytes([fci[0], fci[1]]);
                    let mask = u16::from_be_bytes([fci[2], fci[3]]);
                    for bit in 0..17u16 {
                        if bit == 0 || mask & (1 << (bit - 1)) != 0 {
                            let sequence = pid.wrapping_add(bit);
                            if feedback.nacks.len() < 128 && !feedback.nacks.contains(&sequence) {
                                feedback.nacks.push(sequence);
                            }
                        }
                    }
                }
            }
            (206, 1) => {
                if body.len() >= 12
                    && u32::from_be_bytes([body[8], body[9], body[10], body[11]]) == media_ssrc
                {
                    feedback.keyframe = true;
                }
            }
            (206, 15) => {
                if body.len() >= 20 && body[12..16] == *b"REMB" {
                    let media = u32::from_be_bytes([body[8], body[9], body[10], body[11]]);
                    let listed = body[20..]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .take(usize::from(body[16]))
                        .map(|ssrc| u32::from_be_bytes(*ssrc))
                        .collect::<Vec<_>>();
                    let ours = if listed.is_empty() {
                        media == 0 || media == media_ssrc
                    } else {
                        listed.contains(&media_ssrc)
                    };
                    if ours {
                        let exponent = body[17] >> 2;
                        let mantissa = (u32::from(body[17] & 0x03) << 16)
                            | (u32::from(body[18]) << 8)
                            | u32::from(body[19]);
                        let bitrate = mantissa.checked_shl(u32::from(exponent))?;
                        feedback.bitrate =
                            Some(feedback.bitrate.map_or(bitrate, |old| old.min(bitrate)));
                    }
                }
            }
            (201, _) => {
                for index in 0..format as usize {
                    let start = 8 + index * 24;
                    let Some(block) = body.get(start..start + 24) else {
                        break;
                    };
                    if u32::from_be_bytes([block[0], block[1], block[2], block[3]]) == media_ssrc {
                        feedback.loss =
                            Some(feedback.loss.map_or(block[4], |old: u8| old.max(block[4])));
                    }
                }
            }
            _ => {}
        }
        offset = end;
    }
    Some(feedback)
}

pub(super) struct SenderStats {
    pub ntp_seconds: u32,
    pub ntp_fraction: u32,
    pub rtp_timestamp: u32,
    pub packet_count: u32,
    pub octet_count: u32,
}

pub(super) fn sender_report(sender: u32, stats: &SenderStats) -> Vec<u8> {
    let mut packet = vec![0x80, 200, 0, 6];
    packet.extend_from_slice(&sender.to_be_bytes());
    packet.extend_from_slice(&stats.ntp_seconds.to_be_bytes());
    packet.extend_from_slice(&stats.ntp_fraction.to_be_bytes());
    packet.extend_from_slice(&stats.rtp_timestamp.to_be_bytes());
    packet.extend_from_slice(&stats.packet_count.to_be_bytes());
    packet.extend_from_slice(&stats.octet_count.to_be_bytes());
    packet
}
