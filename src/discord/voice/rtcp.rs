// RFC 5761 section 4: RTCP packet types occupy 192..=223, outside RTP's 0..=127.
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
                // RFC 3550 section 6.4.1: cumulative lost is a signed 24-bit value.
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

#[cfg(test)]
mod tests {
    use super::*;

    const OWN_SSRC: u32 = 0x1122_3344;

    fn block(ssrc: u32, fraction_lost: u8, cumulative_lost: i32) -> [u8; 24] {
        let mut bytes = [0; 24];
        bytes[..4].copy_from_slice(&ssrc.to_be_bytes());
        bytes[4] = fraction_lost;
        bytes[5..8].copy_from_slice(&cumulative_lost.to_be_bytes()[1..]);
        bytes
    }

    fn receiver_report(blocks: &[[u8; 24]]) -> Vec<u8> {
        let mut packet = vec![0x80 | blocks.len() as u8, 201, 0, 0];
        packet[2..4].copy_from_slice(&(blocks.len() as u16 * 6 + 1).to_be_bytes());
        packet.extend_from_slice(&0u32.to_be_bytes());
        for block in blocks {
            packet.extend_from_slice(block);
        }
        packet
    }

    fn sender_report(blocks: &[[u8; 24]]) -> Vec<u8> {
        let mut packet = vec![0x80 | blocks.len() as u8, 200, 0, 0];
        packet[2..4].copy_from_slice(&(blocks.len() as u16 * 6 + 6).to_be_bytes());
        packet.extend_from_slice(&[0; 24]);
        for block in blocks {
            packet.extend_from_slice(block);
        }
        packet
    }

    #[test]
    fn rtcp_types_are_demultiplexed_from_rtp() {
        assert_eq!(packet_type(&[0x80, 200]), Some(200));
        assert_eq!(packet_type(&[0x80, 204]), Some(204));
        assert_eq!(packet_type(&[0x80, 223]), Some(223));
        assert_eq!(packet_type(&[0x80, 191]), None);
        assert_eq!(packet_type(&[0x80, 224]), None);
        assert_eq!(packet_type(&[0x80, 120]), None);
        assert_eq!(packet_type(&[]), None);
    }

    #[test]
    fn receiver_report_block_for_own_ssrc() {
        let packet = receiver_report(&[block(OWN_SSRC, 64, 3)]);
        let block = report_block_for(&packet, OWN_SSRC).unwrap();
        assert_eq!(block.ssrc, OWN_SSRC);
        assert_eq!(block.fraction_lost, 64);
        assert_eq!(block.cumulative_lost, 3);
        assert_eq!(block.loss_percent(), 25.0);
    }

    #[test]
    fn sender_report_block() {
        let packet = sender_report(&[block(0xAAAA_BBBB, 0, 0), block(OWN_SSRC, 255, 7)]);
        let block = report_block_for(&packet, OWN_SSRC).unwrap();
        assert_eq!(block.fraction_lost, 255);
        assert_eq!(block.cumulative_lost, 7);
        assert!((block.loss_percent() - 99.609_375).abs() < f32::EPSILON);
    }

    #[test]
    fn compound_sender_report_and_sdes() {
        let mut packet = sender_report(&[block(OWN_SSRC, 32, -2)]);
        packet.extend_from_slice(&[0x81, 202, 0, 2, 0, 0, 0, 0, 1, 1, b'x', 0]);
        let block = report_block_for(&packet, OWN_SSRC).unwrap();
        assert_eq!(block.fraction_lost, 32);
        assert_eq!(block.cumulative_lost, -2);
        assert_eq!(block.loss_percent(), 12.5);
    }

    #[test]
    fn block_for_a_different_ssrc_is_ignored() {
        let packet = receiver_report(&[block(0xAAAA_BBBB, 10, 1)]);
        assert!(report_block_for(&packet, OWN_SSRC).is_none());
    }

    #[test]
    fn truncated_input_does_not_panic() {
        assert!(report_block_for(&[], OWN_SSRC).is_none());
        assert!(report_block_for(&[0x80], OWN_SSRC).is_none());
        assert!(report_block_for(&[0x80, 201, 0], OWN_SSRC).is_none());

        let mut packet = receiver_report(&[block(OWN_SSRC, 4, 0)]);
        packet.truncate(packet.len() - 1);
        assert!(report_block_for(&packet, OWN_SSRC).is_none());

        let mut oversized = vec![0x81, 201, 0, 100];
        oversized.extend_from_slice(&[0; 8]);
        assert!(report_block_for(&oversized, OWN_SSRC).is_none());

        let mut missing_block = sender_report(&[]);
        missing_block[0] |= 1;
        assert!(report_block_for(&missing_block, OWN_SSRC).is_none());
    }
}
