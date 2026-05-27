//! Secure credential storage backed by the OS keyring.
//!
//! On Windows: Credential Manager (DPAPI, per-user encryption).
//! On macOS:   Keychain.
//! On Linux:   secret-service (gnome-keyring / kwallet).
//!
//! Password is stored under service `"MoonIo"` and account name == Moon Io
//! account identifier (e.g. `"aruba"`, `"gmail"`).

use keyring::Entry;

const SERVICE: &str = "MoonIo";

/// Read password for an account. Returns None on any error (missing entry,
/// keyring unavailable, etc.) so callers can fall back to other sources.
pub fn get_password(account: &str) -> Option<String> {
    let entry = Entry::new(SERVICE, account).ok()?;
    entry.get_password().ok()
}

/// Store password for an account. Overwrites any existing entry.
pub fn set_password(account: &str, password: &str) -> Result<(), String> {
    let entry = Entry::new(SERVICE, account)
        .map_err(|e| format!("Keyring init: {e}"))?;
    entry
        .set_password(password)
        .map_err(|e| format!("Keyring set: {e}"))
}

/// Remove credentials for an account. Returns Ok(false) if there was nothing to delete.
pub fn delete_password(account: &str) -> Result<bool, String> {
    let entry = Entry::new(SERVICE, account)
        .map_err(|e| format!("Keyring init: {e}"))?;
    match entry.delete_credential() {
        Ok(_) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(format!("Keyring delete: {e}")),
    }
}

/// True if a password is stored for this account.
pub fn has_password(account: &str) -> bool {
    get_password(account).is_some()
}
