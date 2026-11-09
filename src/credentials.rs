use std::{
    sync::{OnceLock, mpsc},
    thread,
    time::Duration,
};

const SERVICE: &str = "zcord";
const LEGACY_SERVICE: &str = "discord-client";
const LEGACY_USER: &str = "token";
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

fn entry(service: &str, user: &str) -> keyring::Result<keyring::Entry> {
    #[cfg(target_os = "linux")]
    {
        // The 3.6.3 default builder bypasses keyutils even with the persistent feature.
        let credential =
            keyring::keyutils_persistent::KeyutilsPersistentCredential::new_with_target(
                None, service, user,
            )?;
        Ok(keyring::Entry::new_with_credential(Box::new(credential)))
    }
    #[cfg(not(target_os = "linux"))]
    keyring::Entry::new(service, user)
}

fn account_entry(user_id: u64) -> keyring::Result<keyring::Entry> {
    entry(SERVICE, &user_id.to_string())
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

fn wait<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, CredentialError> + Send + 'static,
) -> Result<T, CredentialError> {
    let result = enqueue(work).map_err(|_| CredentialError::Unavailable)?;
    match result.recv_timeout(TIMEOUT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(CredentialError::TimedOut),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(CredentialError::Unavailable),
    }
}

fn read(entry: keyring::Result<keyring::Entry>) -> Result<Option<String>, CredentialError> {
    match entry.and_then(|entry| entry.get_password()) {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn load(user_id: u64) -> Result<Option<String>, CredentialError> {
    wait(move || read(account_entry(user_id)))
}

pub fn load_legacy() -> Result<Option<String>, CredentialError> {
    wait(|| match read(entry(SERVICE, LEGACY_USER))? {
        Some(token) => Ok(Some(token)),
        None => read(entry(LEGACY_SERVICE, LEGACY_USER)),
    })
}

pub fn save(user_id: u64, token: &str) -> anyhow::Result<()> {
    let token = token.to_owned();
    queue_write("save", move || {
        account_entry(user_id)?.set_password(&token)?;
        clear_legacy_now()
    })
}

pub fn clear_legacy() -> anyhow::Result<()> {
    queue_write("clear", clear_legacy_now)
}

pub fn clear(user_id: u64) -> anyhow::Result<()> {
    queue_write("clear", move || {
        delete(account_entry(user_id))?;
        clear_legacy_now()
    })
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

fn delete(entry: keyring::Result<keyring::Entry>) -> keyring::Result<()> {
    match entry.and_then(|entry| entry.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error),
    }
}

fn clear_legacy_now() -> keyring::Result<()> {
    let mut result = Ok(());
    for service in [SERVICE, LEGACY_SERVICE] {
        if let Err(error) = delete(entry(service, LEGACY_USER)) {
            result = Err(error);
        }
    }
    result
}
