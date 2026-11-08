use super::*;

const OWN_SSRC: u32 = 0x1122_3344;

fn block(ssrc: u32, fraction_lost: u8, cumulative_lost: i32) -> [u8; 24] {
    let mut bytes = [0; 24];
    bytes[..4].copy_from_slice(&ssrc.to_be_bytes());
    bytes[4] = fraction_lost;
    bytes[5..8].copy_from_slice(&cumulative_lost.to_be_bytes()[1..]);
    bytes
}

fn receiver_report_fixture(blocks: &[[u8; 24]]) -> Vec<u8> {
    let mut packet = vec![0x80 | blocks.len() as u8, 201, 0, 0];
    packet[2..4].copy_from_slice(&(blocks.len() as u16 * 6 + 1).to_be_bytes());
    packet.extend_from_slice(&0u32.to_be_bytes());
    for block in blocks {
        packet.extend_from_slice(block);
    }
    packet
}

fn sender_report_fixture(blocks: &[[u8; 24]]) -> Vec<u8> {
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
    let packet = receiver_report_fixture(&[block(OWN_SSRC, 64, 3)]);
    let block = report_block_for(&packet, OWN_SSRC).unwrap();
    assert_eq!(block.ssrc, OWN_SSRC);
    assert_eq!(block.fraction_lost, 64);
    assert_eq!(block.cumulative_lost, 3);
    assert_eq!(block.loss_percent(), 25.0);
}

#[test]
fn sender_report_block() {
    let packet = sender_report_fixture(&[block(0xAAAA_BBBB, 0, 0), block(OWN_SSRC, 255, 7)]);
    let block = report_block_for(&packet, OWN_SSRC).unwrap();
    assert_eq!(block.fraction_lost, 255);
    assert_eq!(block.cumulative_lost, 7);
    assert!((block.loss_percent() - 99.609_375).abs() < f32::EPSILON);
}

#[test]
fn compound_sender_report_and_sdes() {
    let mut packet = sender_report_fixture(&[block(OWN_SSRC, 32, -2)]);
    packet.extend_from_slice(&[0x81, 202, 0, 2, 0, 0, 0, 0, 1, 1, b'x', 0]);
    let block = report_block_for(&packet, OWN_SSRC).unwrap();
    assert_eq!(block.fraction_lost, 32);
    assert_eq!(block.cumulative_lost, -2);
    assert_eq!(block.loss_percent(), 12.5);
}

#[test]
fn block_for_a_different_ssrc_is_ignored() {
    let packet = receiver_report_fixture(&[block(0xAAAA_BBBB, 10, 1)]);
    assert!(report_block_for(&packet, OWN_SSRC).is_none());
}

#[test]
fn truncated_input_does_not_panic() {
    assert!(report_block_for(&[], OWN_SSRC).is_none());
    assert!(report_block_for(&[0x80], OWN_SSRC).is_none());
    assert!(report_block_for(&[0x80, 201, 0], OWN_SSRC).is_none());

    let mut packet = receiver_report_fixture(&[block(OWN_SSRC, 4, 0)]);
    packet.truncate(packet.len() - 1);
    assert!(report_block_for(&packet, OWN_SSRC).is_none());

    let mut oversized = vec![0x81, 201, 0, 100];
    oversized.extend_from_slice(&[0; 8]);
    assert!(report_block_for(&oversized, OWN_SSRC).is_none());

    let mut missing_block = sender_report_fixture(&[]);
    missing_block[0] |= 1;
    assert!(report_block_for(&missing_block, OWN_SSRC).is_none());
}

#[test]
fn generic_nack_groups_lost_packets_into_fcis() {
    assert_eq!(
        generic_nack(9, 42, &[5]),
        [0x81, 205, 0, 3, 0, 0, 0, 9, 0, 0, 0, 42, 0, 5, 0, 0]
    );
    assert_eq!(
        generic_nack(9, 42, &[5, 6, 7]),
        [0x81, 205, 0, 3, 0, 0, 0, 9, 0, 0, 0, 42, 0, 5, 0, 3]
    );
    let sequences: Vec<_> = (1..=18).collect();
    let packet = generic_nack(9, 42, &sequences);
    assert_eq!(&packet[2..4], &[0, 4]);
    assert_eq!(&packet[12..16], &[0, 1, 0xff, 0xff]);
    assert_eq!(&packet[16..20], &[0, 18, 0, 0]);
}

#[test]
fn pli_and_receiver_report_have_rfc_shapes() {
    assert_eq!(pli(9, 42), [0x81, 206, 0, 2, 0, 0, 0, 9, 0, 0, 0, 42]);
    let report = ReceiverReport {
        media_ssrc: 42,
        fraction_lost: 32,
        cumulative_lost: 3,
        highest_sequence: 7,
        jitter: 8,
        lsr: 9,
        dlsr: 10,
    };
    assert_eq!(
        receiver_report(9, &report),
        [
            0x81, 201, 0, 7, 0, 0, 0, 9, 0, 0, 0, 42, 32, 0, 0, 3, 0, 0, 0, 7, 0, 0, 0, 8, 0, 0, 0,
            9, 0, 0, 0, 10
        ]
    );
}

fn feedback_packet(kind: u8, format: u8, body: &[u8]) -> Vec<u8> {
    let mut packet = vec![0x80 | format, kind];
    packet.extend_from_slice(&((body.len() / 4) as u16).to_be_bytes());
    packet.extend_from_slice(body);
    packet
}

#[test]
fn nack_pli_and_remb_are_parsed() {
    let mut nack = 9u32.to_be_bytes().to_vec();
    nack.extend_from_slice(&42u32.to_be_bytes());
    nack.extend_from_slice(&[0x00, 0x05, 0x00, 0x03]);
    let parsed = feedback(&feedback_packet(205, 1, &nack), 42).unwrap();
    assert_eq!(parsed.nacks, [5, 6, 7]);
    assert!(!parsed.keyframe);

    let pli = [9u32.to_be_bytes(), 42u32.to_be_bytes()].concat();
    assert!(
        feedback(&feedback_packet(206, 1, &pli), 42)
            .unwrap()
            .keyframe
    );

    // 6 Mbps for SSRC 42: exponent 5, mantissa 187500, after a count of one SSRC.
    let remb_for = |ssrc: u32| {
        let mut remb = 9u32.to_be_bytes().to_vec();
        remb.extend_from_slice(&0u32.to_be_bytes());
        remb.extend_from_slice(b"REMB");
        remb.extend_from_slice(&[1, (5 << 2) | 0x02, 0xdc, 0x6c]);
        remb.extend_from_slice(&ssrc.to_be_bytes());
        feedback(&feedback_packet(206, 15, &remb), 42)
            .unwrap()
            .bitrate
    };
    assert_eq!(remb_for(42), Some(6_000_000));
    assert_eq!(remb_for(7), None);
}

#[test]
fn sender_report_has_the_rfc3550_shape() {
    let stats = SenderStats {
        ntp_seconds: 0x0102_0304,
        ntp_fraction: 0x0506_0708,
        rtp_timestamp: 0x090A_0B0C,
        packet_count: 0x0D0E_0F10,
        octet_count: 0x1112_1314,
    };
    assert_eq!(
        sender_report(9, &stats),
        [
            0x80, 200, 0, 6, 0, 0, 0, 9, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17,
            18, 19, 20
        ]
    );
}

#[test]
fn sender_report_lsr_uses_the_middle_ntp_bits() {
    let mut packet = vec![0x80, 200, 0, 6];
    packet.extend_from_slice(&42u32.to_be_bytes());
    packet.extend_from_slice(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
    packet.extend_from_slice(&[0; 20]);
    assert_eq!(sender_report_lsr(&packet, 42), Some(0x0304_0506));
    assert_eq!(sender_report_lsr(&packet, 43), None);
    assert_eq!(sender_report_lsr(&packet[..20], 42), None);
}
