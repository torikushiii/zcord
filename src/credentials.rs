use std::{
    sync::{OnceLock, mpsc},
    thread,
    time::Duration,
};

const SERVICE: &str = "zcord";
const LEGACY_SERVICE: &str = "discord-client";
const TIMEOUT: Duration = Duration::from_secs(5);

type Work = Box<dyn FnOnce() + Send>;
static WORKER: OnceLock<std::io::Result<mpsc::Sender<Work>>> = OnceLock::new();

#[derive(Debug)]
pub enum CredentialError {
    Unavailable,
    Locked,
    TimedOut,
}

impl std::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "OS credential store is unavailable",
            Self::Locked => "OS credential store is locked or inaccessible",
            Self::TimedOut => "OS credential store is unavailable (request timed out)",
        })
    }
}

impl std::error::Error for CredentialError {}

impl From<keyring::Error> for CredentialError {
    fn from(error: keyring::Error) -> Self {
        match error {
            keyring::Error::NoStorageAccess(_) => Self::Locked,
            _ => Self::Unavailable,
        }
    }
}

fn entry(service: &str) -> keyring::Result<keyring::Entry> {
    #[cfg(target_os = "linux")]
    {
        // The 3.6.3 default builder bypasses keyutils even with the persistent feature.
        let credential =
            keyring::keyutils_persistent::KeyutilsPersistentCredential::new_with_target(
                None, service, "token",
            )?;
        Ok(keyring::Entry::new_with_credential(Box::new(credential)))
    }
    #[cfg(not(target_os = "linux"))]
    keyring::Entry::new(service, "token")
}

// Keep reads, migration and mutations in order, including work that outlives its timeout.
fn enqueue<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> anyhow::Result<mpsc::Receiver<T>> {
    let worker = WORKER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<Work>();
        thread::Builder::new()
            .name("credentials".into())
            .spawn(move || {
                for work in receiver {
                    work();
                }
            })?;
        Ok(sender)
    });
    let worker = worker
        .as_ref()
        .map_err(|_| anyhow::anyhow!("OS credential store worker is unavailable"))?;
    let (sender, receiver) = mpsc::channel();
    worker
        .send(Box::new(move || {
            let _ = sender.send(work());
        }))
        .map_err(|_| anyhow::anyhow!("OS credential store worker is unavailable"))?;
    Ok(receiver)
}

pub fn load() -> Result<Option<String>, CredentialError> {
    let result = enqueue(load_now).map_err(|_| CredentialError::Unavailable)?;
    match result.recv_timeout(TIMEOUT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(CredentialError::TimedOut),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(CredentialError::Unavailable),
    }
}

fn load_now() -> Result<Option<String>, CredentialError> {
    match entry(SERVICE).and_then(|entry| entry.get_password()) {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => migrate_legacy(),
        Err(error) => Err(error.into()),
    }
}

fn migrate_legacy() -> Result<Option<String>, CredentialError> {
    let legacy = entry(LEGACY_SERVICE)?;
    let token = match legacy.get_password() {
        Ok(token) => token,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if save_now(&token).is_ok() {
        if let Err(error) = legacy.delete_credential()
            && !matches!(error, keyring::Error::NoEntry)
        {
            log::warn!("OS credential store could not clear legacy credential");
        }
    } else {
        log::warn!("OS credential store could not migrate legacy credential");
    }
    Ok(Some(token))
}

pub fn save(token: &str) -> anyhow::Result<()> {
    let token = token.to_owned();
    queue_write("save", move || save_now(&token))
}

pub fn clear() -> anyhow::Result<()> {
    queue_write("clear", clear_now)
}

fn queue_write(
    operation: &'static str,
    work: impl FnOnce() -> keyring::Result<()> + Send + 'static,
) -> anyhow::Result<()> {
    enqueue(move || {
        if work().is_err() {
            log::warn!("OS credential store could not {operation} credential");
        }
    })?;
    Ok(())
}

fn save_now(token: &str) -> keyring::Result<()> {
    entry(SERVICE)?.set_password(token)
}

fn clear_now() -> keyring::Result<()> {
    let mut result = Ok(());
    for service in [SERVICE, LEGACY_SERVICE] {
        match entry(service).and_then(|entry| entry.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(error) => result = Err(error),
        }
    }
    result
}
