use davey::{DAVE_PROTOCOL_VERSION, DaveSession, MediaType, ProposalsOperationType};
use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU16,
    time::{Duration, Instant},
};

pub(super) const SILENCE: [u8; 3] = [0xF8, 0xFF, 0xFE];

pub(super) enum EncryptOutcome {
    Plaintext(Vec<u8>),
    Encrypted(Vec<u8>),
    NotReady,
    Failed,
}

#[derive(Debug, PartialEq, Eq)]
enum EncryptPlan {
    Plaintext,
    Encrypt,
    Drop,
}

fn encryption_plan(version: u16, ready: bool) -> EncryptPlan {
    if version == 0 {
        EncryptPlan::Plaintext
    } else if ready {
        EncryptPlan::Encrypt
    } else {
        EncryptPlan::Drop
    }
}

pub(super) struct Dave {
    session: DaveSession,
    version: u16,
    user_id: u64,
    channel_id: u64,
    external_sender: Option<Vec<u8>>,
    pending: HashMap<u16, u16>,
    recognized: HashSet<u64>,
    active_version: u16,
    passthrough: bool,
    passthrough_until: Option<Instant>,
    encrypting_logged: bool,
}

impl Dave {
    pub fn new(version: u16, user_id: u64, channel_id: u64) -> Option<Self> {
        if version == 0 || version > DAVE_PROTOCOL_VERSION {
            return None;
        }
        let session =
            DaveSession::new(NonZeroU16::new(version)?, user_id, channel_id, None).ok()?;
        let mut recognized = HashSet::new();
        recognized.insert(user_id);
        Some(Self {
            session,
            version,
            user_id,
            channel_id,
            external_sender: None,
            pending: HashMap::new(),
            recognized,
            active_version: version,
            passthrough: false,
            passthrough_until: None,
            encrypting_logged: false,
        })
    }

    pub fn version(&self) -> u16 {
        self.version
    }

    pub fn is_ready(&self) -> bool {
        self.session.is_ready()
    }

    pub fn transition_version(&self, transition_id: u16) -> Option<u16> {
        self.pending.get(&transition_id).copied()
    }

    pub fn mark_upgrade(&mut self) {
        self.active_version = 0;
    }

    pub fn add_user(&mut self, user_id: u64) {
        self.recognized.insert(user_id);
    }

    pub fn remove_user(&mut self, user_id: u64) {
        self.recognized.remove(&user_id);
    }

    pub fn key_package(&mut self) -> Option<Vec<u8>> {
        self.session.create_key_package().ok()
    }

    pub fn set_external_sender(&mut self, data: &[u8]) -> Option<()> {
        self.external_sender = Some(data.to_vec());
        self.session.set_external_sender(data).ok()
    }

    pub fn proposals(&mut self, operation: u8, data: &[u8]) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
        let operation = match operation {
            0 => ProposalsOperationType::APPEND,
            1 => ProposalsOperationType::REVOKE,
            _ => return None,
        };
        let expected: Vec<_> = self.recognized.iter().copied().collect();
        let result = self
            .session
            .process_proposals(operation, data, Some(&expected))
            .ok()??;
        Some((result.commit, result.welcome))
    }

    pub fn commit(&mut self, transition_id: u16, data: &[u8]) -> bool {
        if self.session.process_commit(data).is_err() {
            return false;
        }
        self.pending.entry(transition_id).or_insert(self.version);
        true
    }

    pub fn welcome(&mut self, transition_id: u16, data: &[u8]) -> bool {
        if self.session.process_welcome(data).is_err() {
            return false;
        }
        self.pending.entry(transition_id).or_insert(self.version);
        true
    }

    pub fn prepare_transition(&mut self, version: u16, transition_id: u16) -> bool {
        self.pending.insert(transition_id, version);
        if version == 0 {
            self.passthrough = true;
            self.passthrough_until = None;
            self.session.set_passthrough_mode(true, Some(120));
        }
        true
    }

    pub fn execute_transition(&mut self, transition_id: u16) -> Option<Option<String>> {
        let version = self.pending.remove(&transition_id)?;
        let previous = self.active_version;
        if version == 0 {
            self.version = 0;
            self.active_version = 0;
            self.passthrough = true;
            self.passthrough_until = None;
            return Some(None);
        }
        if version != self.version {
            self.version = version;
        }
        if self.session.is_ready() {
            self.active_version = version;
            if previous == 0 {
                self.passthrough = true;
                self.passthrough_until = Some(Instant::now() + Duration::from_secs(10));
                self.session.set_passthrough_mode(true, Some(10));
            } else {
                self.passthrough = false;
                self.passthrough_until = None;
            }
            return Some(self.privacy_code());
        }
        None
    }

    pub fn prepare_epoch(&mut self, version: u16, epoch: u64) -> Option<Vec<u8>> {
        if epoch != 1 || version == 0 || version > DAVE_PROTOCOL_VERSION {
            return None;
        }
        let version = NonZeroU16::new(version)?;
        self.session
            .reinit(version, self.user_id, self.channel_id, None)
            .ok()?;
        self.version = version.get();
        self.pending.clear();
        self.passthrough = false;
        self.passthrough_until = None;
        if let Some(external_sender) = &self.external_sender {
            self.session.set_external_sender(external_sender).ok()?;
        }
        self.session.create_key_package().ok()
    }

    pub fn encrypt(&mut self, packet: &[u8]) -> EncryptOutcome {
        match encryption_plan(self.version, self.session.is_ready()) {
            EncryptPlan::Plaintext => EncryptOutcome::Plaintext(packet.to_vec()),
            EncryptPlan::Drop => EncryptOutcome::NotReady,
            EncryptPlan::Encrypt => match self.session.encrypt_opus(packet) {
                Ok(encrypted) => {
                    if !self.encrypting_logged {
                        self.encrypting_logged = true;
                        log::info!("DAVE encrypting: version={} ready=true", self.version);
                    }
                    EncryptOutcome::Encrypted(encrypted.into_owned())
                }
                Err(_) => EncryptOutcome::Failed,
            },
        }
    }

    pub fn decrypt(&mut self, user_id: u64, packet: &[u8]) -> Option<Vec<u8>> {
        if packet == SILENCE {
            return Some(packet.to_vec());
        }
        if self.version == 0 {
            return self.passthrough.then(|| packet.to_vec());
        }
        if !self.session.is_ready() {
            return self.passthrough_allowed().then(|| packet.to_vec());
        }
        match self.session.decrypt(user_id, MediaType::AUDIO, packet) {
            Ok(plaintext) => Some(plaintext),
            Err(_) if self.passthrough_allowed() => Some(packet.to_vec()),
            Err(_) => None,
        }
    }

    fn passthrough_allowed(&self) -> bool {
        self.passthrough
            && self
                .passthrough_until
                .is_none_or(|deadline| Instant::now() < deadline)
    }

    pub fn privacy_code(&self) -> Option<String> {
        self.session.voice_privacy_code().map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_one_starts_not_ready_then_downgrade_is_plaintext() {
        let mut dave = Dave::new(1, 42, 99).unwrap();
        assert!(Dave::new(2, 42, 99).is_none());
        assert!(matches!(dave.encrypt(&SILENCE), EncryptOutcome::NotReady));
        assert_eq!(dave.decrypt(7, &SILENCE).unwrap(), SILENCE);
        assert!(matches!(dave.encrypt(&[1, 2, 3]), EncryptOutcome::NotReady));
        assert!(dave.decrypt(7, &[1, 2, 3]).is_none());
        assert!(dave.execute_transition(9).is_none());
        assert!(dave.prepare_transition(0, 4));
        assert_eq!(dave.execute_transition(4), Some(None));
        assert_eq!(dave.version(), 0);
        assert!(matches!(
            dave.encrypt(&[4, 5, 6]),
            EncryptOutcome::Plaintext(_)
        ));
        assert!(dave.decrypt(7, &[4, 5, 6]).is_some());
    }

    #[test]
    fn transition_version_tracks_the_prepared_transition() {
        let mut dave = Dave::new(1, 42, 99).unwrap();
        assert!(dave.prepare_transition(2, 7));
        assert_eq!(dave.transition_version(7), Some(2));
        assert!(dave.execute_transition(7).is_none());
        assert_eq!(dave.transition_version(7), None);
    }

    #[test]
    fn execute_transition_records_new_version_before_readiness() {
        let mut dave = Dave::new(1, 42, 99).unwrap();
        assert!(dave.prepare_transition(2, 7));
        assert!(dave.execute_transition(7).is_none());
        assert_eq!(dave.version(), 2);
        assert!(matches!(dave.encrypt(&[1, 2, 3]), EncryptOutcome::NotReady));
    }

    #[test]
    fn encryption_plan_follows_version_and_readiness() {
        assert_eq!(encryption_plan(0, false), EncryptPlan::Plaintext);
        assert_eq!(encryption_plan(0, true), EncryptPlan::Plaintext);
        assert_eq!(encryption_plan(1, true), EncryptPlan::Encrypt);
        assert_eq!(encryption_plan(1, false), EncryptPlan::Drop);
    }
}
