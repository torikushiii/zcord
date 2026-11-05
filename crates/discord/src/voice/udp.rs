pub(super) const DISCOVERY_LEN: usize = 74;
const DISCOVERY_REQUEST: u16 = 1;
const DISCOVERY_RESPONSE: u16 = 2;
const ADDRESS_LEN: usize = 64;

pub(super) fn discovery_request(ssrc: u32) -> [u8; DISCOVERY_LEN] {
    let mut packet = [0; DISCOVERY_LEN];
    packet[0..2].copy_from_slice(&DISCOVERY_REQUEST.to_be_bytes());
    packet[2..4].copy_from_slice(&70u16.to_be_bytes());
    packet[4..8].copy_from_slice(&ssrc.to_be_bytes());
    packet
}

pub(super) fn parse_discovery_response(packet: &[u8]) -> Option<(String, u16)> {
    if packet.len() < DISCOVERY_LEN
        || u16::from_be_bytes([packet[0], packet[1]]) != DISCOVERY_RESPONSE
    {
        return None;
    }
    let address = &packet[8..8 + ADDRESS_LEN];
    let end = address
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(ADDRESS_LEN);
    let address = std::str::from_utf8(&address[..end]).ok()?.to_owned();
    if address.is_empty() {
        return None;
    }
    let port = u16::from_be_bytes([packet[72], packet[73]]);
    (port != 0).then_some((address, port))
}

#[cfg(test)]
pub(super) fn discovery_response(address: &str, port: u16) -> [u8; DISCOVERY_LEN] {
    let mut packet = [0; DISCOVERY_LEN];
    packet[0..2].copy_from_slice(&DISCOVERY_RESPONSE.to_be_bytes());
    packet[2..4].copy_from_slice(&70u16.to_be_bytes());
    let address = address.as_bytes();
    let len = address.len().min(ADDRESS_LEN - 1);
    packet[8..8 + len].copy_from_slice(&address[..len]);
    packet[72..74].copy_from_slice(&port.to_be_bytes());
    packet
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_round_trip() {
        let request = discovery_request(0xDEAD_BEEF);
        assert_eq!(request.len(), 74);
        assert_eq!(&request[..2], &[0, 1]);
        assert_eq!(&request[2..4], &[0, 70]);
        assert_eq!(&request[4..8], &0xDEAD_BEEFu32.to_be_bytes());
        assert!(request[8..].iter().all(|byte| *byte == 0));

        let response = discovery_response("203.0.113.7", 50000);
        assert_eq!(
            parse_discovery_response(&response),
            Some(("203.0.113.7".into(), 50000))
        );
        let mut malformed = response;
        malformed[0] = 1;
        assert!(parse_discovery_response(&malformed).is_none());
        assert!(parse_discovery_response(&response[..73]).is_none());
    }
}
