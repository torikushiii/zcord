#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Header {
    pub version: u8,
    pub padding: bool,
    pub extension: bool,
    pub csrc_count: u8,
    pub marker: bool,
    pub payload_type: u8,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PacketInfo {
    pub header: Header,
    /// Fixed header, CSRCs, and the four-byte extension preamble; the AEAD AAD.
    pub aad_len: usize,
    /// Start of the Opus payload after encrypted extension elements.
    pub payload_offset: usize,
}

impl Header {
    pub fn audio(ssrc: u32, sequence: u16, timestamp: u32, marker: bool) -> Self {
        Self {
            version: 2,
            marker,
            payload_type: 120,
            sequence,
            timestamp,
            ssrc,
            ..Self::default()
        }
    }

    pub fn encode(&self) -> [u8; 12] {
        let mut bytes = [0; 12];
        bytes[0] = (self.version << 6)
            | (u8::from(self.padding) << 5)
            | (u8::from(self.extension) << 4)
            | (self.csrc_count & 0x0F);
        bytes[1] = (u8::from(self.marker) << 7) | (self.payload_type & 0x7F);
        bytes[2..4].copy_from_slice(&self.sequence.to_be_bytes());
        bytes[4..8].copy_from_slice(&self.timestamp.to_be_bytes());
        bytes[8..12].copy_from_slice(&self.ssrc.to_be_bytes());
        bytes
    }

    pub fn parse(packet: &[u8]) -> Option<PacketInfo> {
        if packet.len() < 12 {
            return None;
        }
        let header = Self {
            version: packet[0] >> 6,
            padding: packet[0] & 0x20 != 0,
            extension: packet[0] & 0x10 != 0,
            csrc_count: packet[0] & 0x0F,
            marker: packet[1] & 0x80 != 0,
            payload_type: packet[1] & 0x7F,
            sequence: u16::from_be_bytes([packet[2], packet[3]]),
            timestamp: u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]),
            ssrc: u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]),
        };
        if header.version != 2 {
            return None;
        }
        let csrc_end = 12 + usize::from(header.csrc_count) * 4;
        if packet.len() < csrc_end {
            return None;
        }
        let (aad_len, extension_len) = if header.extension {
            if packet.len() < csrc_end + 4 {
                return None;
            }
            let words = u16::from_be_bytes([packet[csrc_end + 2], packet[csrc_end + 3]]) as usize;
            (csrc_end + 4, words * 4)
        } else {
            (csrc_end, 0)
        };
        let payload_offset = aad_len.checked_add(extension_len)?;
        if packet.len() < payload_offset {
            return None;
        }
        Some(PacketInfo {
            header,
            aad_len,
            payload_offset,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trips_with_csrcs_and_extension() {
        let header = Header {
            version: 2,
            extension: true,
            csrc_count: 2,
            marker: true,
            payload_type: 120,
            sequence: 0x1234,
            timestamp: 0x0102_0304,
            ssrc: 0x0A0B_0C0D,
            ..Header::default()
        };
        let mut packet = header.encode().to_vec();
        packet.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        packet.extend_from_slice(&[0xBE, 0xDE, 0x00, 0x02]);
        packet.extend_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);
        packet.extend_from_slice(&[0xF8, 0xFF, 0xFE]);

        let info = Header::parse(&packet).unwrap();
        assert_eq!(info.header, header);
        assert_eq!(info.aad_len, 12 + 8 + 4);
        assert_eq!(info.payload_offset, info.aad_len + 8);
        assert_eq!(&packet[info.payload_offset..], &[0xF8, 0xFF, 0xFE]);
    }

    #[test]
    fn rejects_truncated_packets() {
        assert!(Header::parse(&[0x80; 11]).is_none());
        let mut packet = Header::audio(1, 2, 3, false).encode().to_vec();
        packet[0] |= 0x0F;
        assert!(Header::parse(&packet).is_none());
        let mut packet = Header::audio(1, 2, 3, false).encode().to_vec();
        packet[0] |= 0x10;
        assert!(Header::parse(&packet).is_none());
        let mut packet = Header::audio(1, 2, 3, false).encode().to_vec();
        packet[0] = (2 << 6) | 0x10;
        packet.extend_from_slice(&[0xBE, 0xDE, 0x00, 0x02]);
        assert!(Header::parse(&packet).is_none());
    }
}
