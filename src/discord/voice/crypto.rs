use aes_gcm::{
    Aes256Gcm, Nonce, Tag,
    aead::{AeadInPlace, KeyInit},
};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use std::num::Wrapping;

const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
const NONCE_SUFFIX_LEN: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Aes256Gcm,
    XChaCha20Poly1305,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Aes256Gcm => "aead_aes256_gcm_rtpsize",
            Self::XChaCha20Poly1305 => "aead_xchacha20_poly1305_rtpsize",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "aead_aes256_gcm_rtpsize" => Some(Self::Aes256Gcm),
            "aead_xchacha20_poly1305_rtpsize" => Some(Self::XChaCha20Poly1305),
            _ => None,
        }
    }

    pub fn preferred(modes: &[String]) -> Option<Self> {
        let offered = |mode: Mode| modes.iter().any(|name| name == mode.name());
        [Self::Aes256Gcm, Self::XChaCha20Poly1305]
            .into_iter()
            .find(|mode| offered(*mode))
    }
}

pub(super) struct Cipher {
    mode: Mode,
    key: [u8; KEY_LEN],
    nonce: Wrapping<u32>,
}

impl Cipher {
    pub fn new(mode: Mode, key: &[u8]) -> Option<Self> {
        if key.len() != KEY_LEN {
            return None;
        }
        let mut bytes = [0; KEY_LEN];
        bytes.copy_from_slice(key);
        Some(Self {
            mode,
            key: bytes,
            nonce: Wrapping(rand::random()),
        })
    }

    // https://docs.discord.food/topics/voice-connections#transport-encryption
    fn nonce(&self, suffix: u32) -> ([u8; 24], usize) {
        let mut nonce = [0; 24];
        nonce[..NONCE_SUFFIX_LEN].copy_from_slice(&suffix.to_be_bytes());
        (nonce, 12)
    }

    pub fn encrypt(&mut self, aad: &[u8], payload: &[u8]) -> Option<Vec<u8>> {
        let suffix = self.nonce.0;
        self.nonce += Wrapping(1);
        let (nonce, nonce_len) = self.nonce(suffix);
        let mut body = payload.to_vec();
        match self.mode {
            Mode::Aes256Gcm => {
                let cipher = Aes256Gcm::new_from_slice(&self.key).ok()?;
                let tag = cipher
                    .encrypt_in_place_detached(
                        Nonce::from_slice(&nonce[..nonce_len]),
                        aad,
                        &mut body,
                    )
                    .ok()?;
                body.extend_from_slice(&tag);
            }
            Mode::XChaCha20Poly1305 => {
                let cipher = XChaCha20Poly1305::new_from_slice(&self.key).ok()?;
                let tag = cipher
                    .encrypt_in_place_detached(XNonce::from_slice(&nonce), aad, &mut body)
                    .ok()?;
                body.extend_from_slice(&tag);
            }
        }
        body.extend_from_slice(&suffix.to_be_bytes());
        Some(body)
    }

    pub fn decrypt(&self, aad: &[u8], body: &[u8]) -> Option<Vec<u8>> {
        if body.len() < NONCE_SUFFIX_LEN + TAG_LEN {
            return None;
        }
        let (ciphertext, suffix) = body.split_at(body.len() - NONCE_SUFFIX_LEN);
        let suffix = u32::from_be_bytes(suffix.try_into().ok()?);
        let (data, tag) = ciphertext.split_at(ciphertext.len() - TAG_LEN);
        let (nonce, nonce_len) = self.nonce(suffix);
        let mut plaintext = data.to_vec();
        match self.mode {
            Mode::Aes256Gcm => {
                let cipher = Aes256Gcm::new_from_slice(&self.key).ok()?;
                cipher
                    .decrypt_in_place_detached(
                        Nonce::from_slice(&nonce[..nonce_len]),
                        aad,
                        &mut plaintext,
                        Tag::from_slice(tag),
                    )
                    .ok()?;
            }
            Mode::XChaCha20Poly1305 => {
                let cipher = XChaCha20Poly1305::new_from_slice(&self.key).ok()?;
                cipher
                    .decrypt_in_place_detached(
                        XNonce::from_slice(&nonce),
                        aad,
                        &mut plaintext,
                        chacha20poly1305::Tag::from_slice(tag),
                    )
                    .ok()?;
            }
        }
        Some(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::voice::rtp::Header;

    #[test]
    fn rtpsize_round_trip_for_both_modes_with_extension() {
        for mode in [Mode::Aes256Gcm, Mode::XChaCha20Poly1305] {
            let mut cipher = Cipher::new(mode, &[7; KEY_LEN]).unwrap();
            let header = Header {
                version: 2,
                extension: true,
                csrc_count: 1,
                payload_type: 120,
                ..Header::default()
            };
            let mut aad = header.encode().to_vec();
            aad.extend_from_slice(&[0, 0, 0, 1]);
            aad.extend_from_slice(&[0xBE, 0xDE, 0x00, 0x02]);
            let extension = [1, 2, 3, 4, 5, 6, 7, 8];
            let opus = [9, 10, 11, 12];
            let mut plaintext = extension.to_vec();
            plaintext.extend_from_slice(&opus);

            let first = cipher.encrypt(&aad, &plaintext).unwrap();
            let second = cipher.encrypt(&aad, &plaintext).unwrap();
            assert_ne!(&first[first.len() - 4..], &second[second.len() - 4..]);
            assert_ne!(first, second);

            let decrypted = cipher.decrypt(&aad, &first).unwrap();
            assert_eq!(decrypted, plaintext);
            assert_eq!(&decrypted[extension.len()..], &opus);
            let decrypted = cipher.decrypt(&aad, &second).unwrap();
            assert_eq!(decrypted, plaintext);
        }
    }


    #[test]
    fn outgoing_packet_decrypts_with_independent_aes_gcm_rtpsize() {
        const KEY: [u8; KEY_LEN] = [0x42; KEY_LEN];
        let opus = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let mut cipher = Cipher::new(Mode::Aes256Gcm, &KEY).unwrap();
        let header = Header::audio(0x1122_3344, 7, 960, true);
        let aad = header.encode();
        assert_eq!(aad.len(), 12);

        let body = cipher.encrypt(&aad, &opus).unwrap();
        let first_suffix = u32::from_be_bytes(body[body.len() - 4..].try_into().unwrap());
        let mut packet = aad.to_vec();
        packet.extend_from_slice(&body);

        let info = Header::parse(&packet).unwrap();
        assert_eq!(info.aad_len, 12);
        assert_eq!(info.payload_offset, 12);
        assert_eq!(info.header.version, 2);
        assert_eq!(info.header.payload_type, 120);
        assert_eq!(info.header.ssrc, 0x1122_3344);
        assert_eq!(info.header.sequence, 7);
        assert_eq!(info.header.timestamp, 960);
        assert!(info.header.marker);

        let sealed = &packet[12..];
        let (ciphertext_and_tag, suffix) = sealed.split_at(sealed.len() - 4);
        let (ciphertext, tag) = ciphertext_and_tag.split_at(ciphertext_and_tag.len() - 16);
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(suffix);
        let mut plaintext = ciphertext.to_vec();
        let aes = Aes256Gcm::new_from_slice(&KEY).unwrap();
        aes.decrypt_in_place_detached(
            Nonce::from_slice(&nonce),
            &aad,
            &mut plaintext,
            Tag::from_slice(tag),
        )
        .unwrap();
        assert_eq!(plaintext, opus);

        let second = cipher.encrypt(&aad, &opus).unwrap();
        let second_suffix = u32::from_be_bytes(second[second.len() - 4..].try_into().unwrap());
        assert_eq!(second_suffix, first_suffix.wrapping_add(1));
    }

    #[test]
    fn parsed_packet_aad_and_extension_strip_round_trip() {
        for mode in [Mode::Aes256Gcm, Mode::XChaCha20Poly1305] {
            let mut cipher = Cipher::new(mode, &[3; KEY_LEN]).unwrap();
            let header = Header {
                version: 2,
                extension: true,
                csrc_count: 2,
                marker: true,
                payload_type: 120,
                ..Header::default()
            };
            let mut packet = header.encode().to_vec();
            packet.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
            let preamble = [0xBE, 0xDE, 0x00, 0x02];
            packet.extend_from_slice(&preamble);
            let extension = [9, 10, 11, 12, 13, 14, 15, 16];
            let mut plaintext = extension.to_vec();
            plaintext.extend_from_slice(&[0xF8, 0xFF, 0xFE]);
            let encrypted = cipher.encrypt(&packet, &plaintext).unwrap();
            packet.extend_from_slice(&encrypted);

            let info = Header::parse(&packet).unwrap();
            assert_eq!(info.aad_len, 12 + 8 + 4);
            let decrypted = cipher
                .decrypt(&packet[..info.aad_len], &packet[info.aad_len..])
                .unwrap();
            let payload = &decrypted[info.payload_offset - info.aad_len..];
            assert_eq!(payload, [0xF8, 0xFF, 0xFE]);
        }
    }

    #[test]
    fn tampering_and_short_packets_fail() {
        for mode in [Mode::Aes256Gcm, Mode::XChaCha20Poly1305] {
            let mut cipher = Cipher::new(mode, &[1; KEY_LEN]).unwrap();
            let aad = Header::audio(1, 2, 3, false).encode();
            let mut body = cipher.encrypt(&aad, &[4, 5, 6]).unwrap();
            assert!(cipher.decrypt(&aad, &body[..body.len() - 1]).is_none());
            body[0] ^= 1;
            assert!(cipher.decrypt(&aad, &body).is_none());
            assert!(cipher.decrypt(&aad, &[0; 8]).is_none());
        }
        assert!(Cipher::new(Mode::Aes256Gcm, &[0; 31]).is_none());
        assert_eq!(
            Mode::preferred(&[
                "aead_xchacha20_poly1305_rtpsize".into(),
                "aead_aes256_gcm_rtpsize".into()
            ]),
            Some(Mode::Aes256Gcm)
        );
        assert_eq!(
            Mode::preferred(&["aead_xchacha20_poly1305_rtpsize".into()]),
            Some(Mode::XChaCha20Poly1305)
        );
        assert_eq!(Mode::preferred(&["unknown".into()]), None);
    }
}
