const MTU: usize = 1200;
const RTP_HEADER: usize = 12;
const EXTENSION_PREAMBLE: usize = 4;
const EXTENSION_BYTES: usize = 16;
const TRANSPORT_OVERHEAD: usize = 20;
const RTX_OVERHEAD: usize = 2;
const MAX_PAYLOAD: usize =
    MTU - RTP_HEADER - EXTENSION_PREAMBLE - EXTENSION_BYTES - TRANSPORT_OVERHEAD - RTX_OVERHEAD;
pub(crate) const MAX_FRAGMENTS: usize = 2048;
const MAX_FRAME: usize = 2 * 1024 * 1024 + 64 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct Packet {
    pub header: [u8; 12],
    pub preamble: [u8; EXTENSION_PREAMBLE],
    pub elements: Vec<u8>,
    pub payload: Vec<u8>,
}

impl Packet {
    pub fn aad(&self) -> [u8; RTP_HEADER + EXTENSION_PREAMBLE] {
        let mut aad = [0; RTP_HEADER + EXTENSION_PREAMBLE];
        aad[..RTP_HEADER].copy_from_slice(&self.header);
        aad[RTP_HEADER..].copy_from_slice(&self.preamble);
        aad
    }

    pub fn plaintext(&self) -> Vec<u8> {
        let mut plaintext = Vec::with_capacity(self.elements.len() + self.payload.len());
        plaintext.extend_from_slice(&self.elements);
        plaintext.extend_from_slice(&self.payload);
        plaintext
    }

    #[cfg(test)]
    pub fn wire_len(&self) -> usize {
        RTP_HEADER
            + EXTENSION_PREAMBLE
            + self.elements.len().min(EXTENSION_BYTES)
            + self.payload.len()
            + TRANSPORT_OVERHEAD
    }

    #[cfg(test)]
    pub fn marker(&self) -> bool {
        self.header[1] & 0x80 != 0
    }

    pub fn rtx(&self, ssrc: u32, sequence: u16, transport_sequence: &mut u16) -> Self {
        let (preamble, elements) = extensions(*transport_sequence, true);
        *transport_sequence = transport_sequence.wrapping_add(1);
        let mut header = self.header;
        header[0] = 0x90;
        header[1] = 102 | (self.header[1] & 0x80);
        header[2..4].copy_from_slice(&sequence.to_be_bytes());
        header[8..12].copy_from_slice(&ssrc.to_be_bytes());
        let mut payload = Vec::with_capacity(self.payload.len() + 2);
        payload.extend_from_slice(&self.header[2..4]);
        payload.extend_from_slice(&self.payload);
        Self {
            header,
            preamble,
            elements,
            payload,
        }
    }
}

/// RFC 6184.
pub(crate) fn packetize(
    frame: &[u8],
    sequence: &mut u16,
    transport_sequence: &mut u16,
    timestamp: u32,
    ssrc: u32,
) -> Result<Vec<Packet>, &'static str> {
    if frame.is_empty() || frame.len() > MAX_FRAME {
        return Err("DAVE H264 frame exceeds the sharing limit");
    }
    let nalus = annex_b_nalus(frame)?;
    let mut packets = Vec::new();
    for (index, nalu) in nalus.iter().enumerate() {
        let last_nalu = index + 1 == nalus.len();
        if nalu.len() <= MAX_PAYLOAD {
            push(
                &mut packets,
                sequence,
                transport_sequence,
                timestamp,
                ssrc,
                nalu,
                last_nalu,
            )?;
            continue;
        }
        if nalu.len() < 2 {
            return Err("Invalid H264 NAL unit");
        }
        let indicator = (nalu[0] & 0xe0) | 28;
        let kind = nalu[0] & 0x1f;
        let mut offset = 1;
        while offset < nalu.len() {
            let take = (nalu.len() - offset).min(MAX_PAYLOAD - 2);
            let mut payload = Vec::with_capacity(take + 2);
            payload.push(indicator);
            payload.push(
                kind | if offset == 1 { 0x80 } else { 0 }
                    | if offset + take == nalu.len() { 0x40 } else { 0 },
            );
            payload.extend_from_slice(&nalu[offset..offset + take]);
            let last = last_nalu && offset + take == nalu.len();
            push(
                &mut packets,
                sequence,
                transport_sequence,
                timestamp,
                ssrc,
                &payload,
                last,
            )?;
            offset += take;
        }
    }
    if packets.is_empty() {
        return Err("H264 frame has no NAL units");
    }
    Ok(packets)
}

fn push(
    packets: &mut Vec<Packet>,
    sequence: &mut u16,
    transport_sequence: &mut u16,
    timestamp: u32,
    ssrc: u32,
    payload: &[u8],
    marker: bool,
) -> Result<(), &'static str> {
    if packets.len() == MAX_FRAGMENTS {
        return Err("H264 frame exceeds fragment limit");
    }
    let (preamble, elements) = extensions(*transport_sequence, false);
    *transport_sequence = transport_sequence.wrapping_add(1);
    let mut header = [0; RTP_HEADER];
    header[0] = 0x90;
    header[1] = 101 | u8::from(marker) << 7;
    header[2..4].copy_from_slice(&sequence.to_be_bytes());
    header[4..8].copy_from_slice(&timestamp.to_be_bytes());
    header[8..12].copy_from_slice(&ssrc.to_be_bytes());
    *sequence = sequence.wrapping_add(1);
    packets.push(Packet {
        header,
        preamble,
        elements,
        payload: payload.to_vec(),
    });
    Ok(())
}

// Discord's client sends these one-byte extensions; the ids differ from its SDP.
fn extensions(transport_sequence: u16, repaired: bool) -> ([u8; EXTENSION_PREAMBLE], Vec<u8>) {
    let mut elements = Vec::with_capacity(EXTENSION_BYTES);
    elements.push((5 << 4) | 1);
    elements.extend_from_slice(&transport_sequence.to_be_bytes());
    elements.push((6 << 4) | 2);
    elements.extend_from_slice(&[0, 0, 0]);
    elements.push((7 << 4) | 0);
    elements.push(1);
    elements.push(((if repaired { 12 } else { 11 }) << 4) | 2);
    elements.extend_from_slice(b"100");
    while !elements.len().is_multiple_of(4) {
        elements.push(0);
    }
    let words = u16::try_from(elements.len() / 4).unwrap_or(u16::MAX);
    let mut preamble = [0xbe, 0xde, 0, 0];
    preamble[2..].copy_from_slice(&words.to_be_bytes());
    (preamble, elements)
}

fn annex_b_nalus(frame: &[u8]) -> Result<Vec<&[u8]>, &'static str> {
    let mut starts = Vec::new();
    let mut at = 0;
    while at + 3 <= frame.len() {
        let size = if frame[at..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if frame[at..].starts_with(&[0, 0, 1]) {
            3
        } else {
            at += 1;
            continue;
        };
        if starts.len() == MAX_FRAGMENTS {
            return Err("H264 frame exceeds fragment limit");
        }
        starts.push((at, size));
        at += size;
    }
    if starts.is_empty() || starts[0].0 != 0 {
        return Err("H264 frame is not Annex-B");
    }
    let mut nalus = Vec::with_capacity(starts.len());
    for (index, (start, size)) in starts.iter().enumerate() {
        let end = starts.get(index + 1).map_or(frame.len(), |next| next.0);
        if start + size >= end {
            return Err("H264 frame contains an empty NAL unit");
        }
        nalus.push(&frame[start + size..end]);
    }
    Ok(nalus)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::voice::{
        crypto::{Cipher, Mode},
        rtp::Header,
    };

    #[test]
    fn single_nal_packet_has_the_expected_extension_bytes() {
        let mut sequence = 7;
        let mut transport = 9;
        let packets = packetize(
            &[0, 0, 0, 1, 0x65, 1, 2, 3],
            &mut sequence,
            &mut transport,
            90,
            11,
        )
        .unwrap();
        assert_eq!(packets.len(), 1);
        assert_eq!(
            packets[0].header,
            [0x90, 0x65 | 0x80, 0, 7, 0, 0, 0, 90, 0, 0, 0, 11]
        );
        assert_eq!(packets[0].preamble, [0xbe, 0xde, 0, 4]);
        assert_eq!(
            packets[0].elements,
            [
                0x51, 0, 9, 0x62, 0, 0, 0, 0x70, 1, 0xb2, b'1', b'0', b'0', 0, 0, 0
            ]
        );
        assert_eq!(packets[0].payload, [0x65, 1, 2, 3]);
        assert_eq!(sequence, 8);
        assert_eq!(transport, 10);
    }

    #[test]
    fn rtx_replaces_the_rid_extension_and_prefixes_the_original_sequence() {
        let mut sequence = 7;
        let mut transport = 9;
        let packets = packetize(
            &[0, 0, 0, 1, 0x41, 1],
            &mut sequence,
            &mut transport,
            90,
            11,
        )
        .unwrap();
        let mut rtx_transport = 20;
        let rtx = packets[0].rtx(12, 400, &mut rtx_transport);
        assert_eq!(rtx.header[1], 0x80 | 102);
        assert_eq!(&rtx.header[2..4], &400u16.to_be_bytes());
        assert_eq!(&rtx.header[8..12], &12u32.to_be_bytes());
        assert_eq!(rtx.preamble, [0xbe, 0xde, 0, 4]);
        assert_eq!(rtx.elements[9], 0xc2);
        assert_eq!(&rtx.payload[..2], &[0, 7]);
        assert_eq!(&rtx.payload[2..], &[0x41, 1]);
    }

    #[test]
    fn extension_bytes_round_trip_through_the_transport_cipher_and_header_parse() {
        let mut sequence = 7;
        let mut transport = 9;
        let packets = packetize(
            &[0, 0, 0, 1, 0x65, 1, 2, 3],
            &mut sequence,
            &mut transport,
            90,
            11,
        )
        .unwrap();
        let packet = &packets[0];
        let aad = packet.aad();
        let mut cipher = Cipher::new(Mode::Aes256Gcm, &[7; 32]).unwrap();
        let sealed = cipher.encrypt(&aad, &packet.plaintext()).unwrap();
        let mut wire = aad.to_vec();
        wire.extend_from_slice(&sealed);
        let info = Header::parse(&wire).unwrap();
        assert_eq!(info.aad_len, 16);
        assert_eq!(info.payload_offset, 32);
        let decrypted = cipher
            .decrypt(&wire[..info.aad_len], &wire[info.aad_len..])
            .unwrap();
        assert_eq!(decrypted[16..], packet.payload);
    }

    #[test]
    fn large_nal_fragments_with_fu_a_and_one_marker() {
        let mut frame = vec![0, 0, 0, 1, 0x65];
        frame.resize(5 + MAX_PAYLOAD * 2, 7);
        let mut sequence = 0;
        let mut transport = 0;
        let packets = packetize(&frame, &mut sequence, &mut transport, 1, 2).unwrap();
        assert!(packets.len() > 2);
        assert!(
            packets[..packets.len() - 1]
                .iter()
                .all(|packet| !packet.marker())
        );
        assert!(packets.last().unwrap().marker());
        assert_eq!(packets[0].payload[0] & 0x1f, 28);
        assert_ne!(packets[0].payload[1] & 0x80, 0);
        assert!(packets.iter().all(|packet| packet.wire_len() <= MTU));
    }
}
