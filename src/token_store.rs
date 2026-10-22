const SERVICE: &str = "zcord";
const LEGACY_SERVICE: &str = "discord-client";

fn entry() -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, "token")
}

pub fn load() -> Option<String> {
    match entry().and_then(|entry| entry.get_password()) {
        Ok(token) => Some(token),
        Err(keyring::Error::NoEntry) => migrate_legacy(),
        Err(_) => {
            log::warn!("OS credential store could not be read");
            None
        }
    }
}

fn migrate_legacy() -> Option<String> {
    let legacy = keyring::Entry::new(LEGACY_SERVICE, "token").ok()?;
    let token = legacy.get_password().ok()?;
    if save(&token).is_ok() {
        let _ = legacy.delete_credential();
    }
    Some(token)
}

pub fn save(token: &str) -> anyhow::Result<()> {
    entry()
        .and_then(|entry| entry.set_password(token))
        .map_err(|_| anyhow::anyhow!("OS credential store could not save credential"))
}

pub fn clear() -> anyhow::Result<()> {
    match entry().and_then(|entry| entry.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(anyhow::anyhow!(
            "OS credential store could not clear credential"
        )),
    }
}
