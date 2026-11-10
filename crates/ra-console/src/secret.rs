//! The OpenRouter key, in the OS credential store. Windows holds it in Credential Manager,
//! encrypted to the user account, so no plaintext file on disk carries it. The environment
//! variable `OPENROUTER_API_KEY` overrides the store when it is set.

use keyring::Entry;

/// The service and account of the entry in the credential store.
const SERVICE: &str = "rhipper-agent";
const ACCOUNT: &str = "openrouter";

/// The tests of the environment path hold this lock, so parallel tests cannot flip the variable
/// under one another.
pub static ENV_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// Reads the OpenRouter key: the environment first, then the credential store.
pub fn api_key() -> anyhow::Result<String> {
    if let Some(key) = std::env::var("OPENROUTER_API_KEY").ok().filter(|k| !k.trim().is_empty()) {
        return Ok(key);
    }
    let entry = Entry::new(SERVICE, ACCOUNT)?;
    match entry.get_password() {
        Ok(key) if !key.trim().is_empty() => Ok(key),
        Ok(_) => anyhow::bail!("the stored OpenRouter key is empty: set one with key <paste>"),
        Err(keyring::Error::NoEntry) => {
            anyhow::bail!("no OpenRouter key: set one with key <paste>, or export OPENROUTER_API_KEY")
        }
        Err(e) => Err(anyhow::Error::from(e).context("cannot read the key from the credential store")),
    }
}

/// Stores the key in the credential store.
pub fn set_key(key: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!key.trim().is_empty(), "the key is empty");
    let entry = Entry::new(SERVICE, ACCOUNT)?;
    entry.set_password(key.trim()).map_err(|e| anyhow::Error::from(e).context("cannot write the credential store"))
}

/// Removes the stored key.
pub fn clear() -> anyhow::Result<()> {
    let entry = Entry::new(SERVICE, ACCOUNT)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => anyhow::bail!("no key is stored"),
        Err(e) => Err(anyhow::Error::from(e).context("cannot clear the credential store")),
    }
}

/// Whether the credential store holds a key, without reading it.
pub fn has_key() -> bool {
    Entry::new(SERVICE, ACCOUNT).and_then(|e| e.get_password()).is_ok()
}

/// The stored key as a mask: its first and last three characters.
pub fn masked() -> anyhow::Result<String> {
    match api_key() {
        Ok(key) if key.len() > 8 => Ok(format!("{}…{}", &key[..3], &key[key.len() - 3..])),
        Ok(_) => Ok("<short key set>".into()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests;
