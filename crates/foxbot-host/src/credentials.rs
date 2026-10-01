use crate::{HostError, Result};
use zeroize::Zeroizing;

const SERVICE: &str = "io.foxbot.credentials.v1";

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialRef {
    pub id: String,
}
impl CredentialRef {
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || self.id.len() > 96
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(HostError::Config);
        }
        Ok(())
    }
    pub(crate) fn account(&self, purpose: &str) -> Result<String> {
        self.validate()?;
        if !matches!(purpose, "ledger" | "token") {
            return Err(HostError::Config);
        }
        Ok(format!("{purpose}:{}", self.id))
    }
}
/// Deliberately has no Debug/Serialize; no environment/file fallback.
pub struct Secret(Zeroizing<Vec<u8>>);
impl Secret {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        let bytes = Zeroizing::new(bytes);
        if bytes.is_empty() || bytes.len() > 8192 {
            return Err(HostError::CredentialInvalid);
        }
        Ok(Self(bytes))
    }
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn token(&self) -> Result<&str> {
        let value = std::str::from_utf8(&self.0).map_err(|_| HostError::CredentialInvalid)?;
        if value.chars().any(char::is_control) {
            return Err(HostError::CredentialInvalid);
        }
        Ok(value)
    }
    pub fn ledger_key(&self) -> Result<&[u8; 32]> {
        self.bytes()
            .try_into()
            .map_err(|_| HostError::CredentialInvalid)
    }
}
pub trait CredentialStore {
    fn load(&self, reference: &CredentialRef, purpose: &str) -> Result<Secret>;
    /// Caller holds DeviceOwner. Never replace an existing ledger key implicitly.
    fn create(&self, reference: &CredentialRef, purpose: &str, secret: &Secret) -> Result<()>;
}
#[cfg(any(target_os = "macos", test))]
fn keychain_error(code: i32) -> HostError {
    eprintln!(
        "{}",
        serde_json::json!({"event":"keychain_error", "os_status":code})
    );
    match code {
        -25300 => HostError::CredentialMissing,
        -25308 => HostError::CredentialInteractionRequired,
        -25293 => HostError::CredentialAccessDenied,
        _ => HostError::CredentialUnavailable,
    }
}

/// The existing file-based macOS Keychain API has process-wide interaction policy.
/// Serialize our calls and restore the prior policy; never accept an authorization dialog
/// from an unattended read. This changes prompting only, not the item's access control.
#[cfg(target_os = "macos")]
fn without_keychain_ui<T>(operation: impl FnOnce() -> Result<T>) -> Result<T> {
    use security_framework::os::macos::keychain::SecKeychain;
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _serialized = LOCK.lock().map_err(|_| HostError::CredentialUnavailable)?;
    let allowed = SecKeychain::user_interaction_allowed().map_err(|e| keychain_error(e.code()))?;
    let _restore = if allowed {
        Some(SecKeychain::disable_user_interaction().map_err(|e| keychain_error(e.code()))?)
    } else {
        None
    };
    operation()
}

pub struct NativeCredentials;
#[cfg(target_os = "macos")]
impl CredentialStore for NativeCredentials {
    fn load(&self, reference: &CredentialRef, purpose: &str) -> Result<Secret> {
        use security_framework::passwords::{PasswordOptions, generic_password};
        let account = reference.account(purpose)?;
        let bytes = without_keychain_ui(|| {
            generic_password(PasswordOptions::new_generic_password(SERVICE, &account))
                .map_err(|e| keychain_error(e.code()))
        })?;
        Secret::new(bytes)
    }
    fn create(&self, reference: &CredentialRef, purpose: &str, secret: &Secret) -> Result<()> {
        match self.load(reference, purpose) {
            Ok(_) => return Err(HostError::CredentialExists),
            Err(HostError::CredentialMissing) => {}
            Err(e) => return Err(e),
        }
        let account = reference.account(purpose)?;
        without_keychain_ui(|| {
            security_framework::passwords::set_generic_password(SERVICE, &account, secret.bytes())
                .map_err(|e| keychain_error(e.code()))
        })
    }
}
#[cfg(not(target_os = "macos"))]
impl CredentialStore for NativeCredentials {
    fn load(&self, reference: &CredentialRef, purpose: &str) -> Result<Secret> {
        let _ = (SERVICE, reference.account(purpose)?);
        Err(HostError::Unsupported)
    }
    fn create(&self, reference: &CredentialRef, purpose: &str, _: &Secret) -> Result<()> {
        let _ = (SERVICE, reference.account(purpose)?);
        Err(HostError::Unsupported)
    }
}

pub fn generate_ledger_key() -> Result<Secret> {
    let mut bytes = Zeroizing::new(vec![0u8; 32]);
    getrandom::fill(&mut bytes).map_err(|_| HostError::CredentialUnavailable)?;
    Secret::new(bytes.to_vec())
}

/// Explicit, opt-in native probe: only a fresh random FoxBot-owned test item is
/// touched. Existing user items are neither enumerated nor read. Always remove it.
#[cfg(target_os = "macos")]
pub fn native_smoke() -> Result<()> {
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| HostError::CredentialUnavailable)?;
    let id = format!(
        "g1c-probe-{}",
        nonce.iter().map(|v| format!("{v:02x}")).collect::<String>()
    );
    let reference = CredentialRef { id };
    let secret = generate_ledger_key()?;
    NativeCredentials.create(&reference, "ledger", &secret)?;
    let account = reference.account("ledger")?;
    struct Cleanup(Option<String>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Some(account) = &self.0 {
                let _ = security_framework::passwords::delete_generic_password(SERVICE, account);
            }
        }
    }
    let mut guard = Cleanup(Some(account.clone()));
    let loaded = NativeCredentials.load(&reference, "ledger")?;
    if loaded.bytes() != secret.bytes() {
        return Err(HostError::CredentialInvalid);
    }
    if !matches!(
        NativeCredentials.create(&reference, "ledger", &secret),
        Err(HostError::CredentialExists)
    ) {
        return Err(HostError::CredentialInvalid);
    }
    security_framework::passwords::delete_generic_password(SERVICE, &account)
        .map_err(|_| HostError::CredentialUnavailable)?;
    guard.0 = None; // Already removed; never delete another later item.
    if !matches!(
        NativeCredentials.load(&reference, "ledger"),
        Err(HostError::CredentialMissing)
    ) {
        return Err(HostError::CredentialUnavailable);
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub fn native_smoke() -> Result<()> {
    Err(HostError::Unsupported)
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn interaction_required_is_not_missing_and_does_not_authorize_replacement() {
        assert_eq!(
            keychain_error(-25308),
            HostError::CredentialInteractionRequired
        );
        assert_ne!(keychain_error(-25308), HostError::CredentialMissing);
    }

    #[test]
    fn keychain_errors_remain_distinct_without_leaking_secrets() {
        assert_eq!(keychain_error(-25300), HostError::CredentialMissing);
        assert_eq!(keychain_error(-25293), HostError::CredentialAccessDenied);
        assert_ne!(keychain_error(-25293), HostError::CredentialMissing);
        for code in [-128, -1] {
            assert_eq!(keychain_error(code), HostError::CredentialUnavailable);
        }
    }
}
