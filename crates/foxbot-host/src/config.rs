use crate::{HostError, Result, credentials::CredentialRef};
use foxbot_core::Binding;
use foxbot_http::HttpConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StorageConfig {
    Local,
    Protected { key: CredentialRef },
    SyntheticPlaintext,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    pub schema_version: u32,
    pub http: HttpConfig,
    pub token: Option<CredentialRef>,
    pub storage: StorageConfig,
    pub bindings: Vec<Binding>,
    pub tick_ms: u64,
    pub max_jobs: usize,
    pub shutdown_ms: u64,
}
impl HostConfig {
    /// Credential resolution is explicit and completes before the host starts. Missing
    /// keys never create a replacement database, and token lookup never reads env vars.
    pub fn open(
        &self,
        directory: &std::path::Path,
        store: &impl crate::credentials::CredentialStore,
        allow_plaintext_synthetic: bool,
    ) -> Result<(foxbot_core::Runtime, foxbot_http::HttpReplyService)> {
        self.validate()?;
        if matches!(self.storage, StorageConfig::SyntheticPlaintext)
            && (!allow_plaintext_synthetic || self.token.is_some())
        {
            return Err(HostError::Config);
        }
        let token = self
            .token
            .as_ref()
            .map(|r| store.load(r, "token"))
            .transpose()?;
        let bearer = token.as_ref().map(|s| s.token()).transpose()?;
        let service = foxbot_http::HttpReplyService::new(self.http.clone(), bearer)
            .map_err(|_| HostError::Config)?;
        let runtime = match &self.storage {
            StorageConfig::Local => foxbot_core::Runtime::open_local(directory)?,
            StorageConfig::Protected { key } => {
                let secret = store.load(key, "ledger")?;
                foxbot_core::Runtime::open_encrypted(directory, secret.ledger_key()?)?
            }
            StorageConfig::SyntheticPlaintext
                if allow_plaintext_synthetic && self.token.is_none() =>
            {
                foxbot_core::Runtime::open_simulation(directory)?
            }
            StorageConfig::SyntheticPlaintext => return Err(HostError::Config),
        };
        Ok((runtime, service))
    }

    pub fn validate(&self) -> Result<()> {
        self.http.validate().map_err(|_| HostError::Config)?;
        if self.schema_version != 1
            || self.bindings.is_empty()
            || self.bindings.len() > 32
            || !(10..=1000).contains(&self.tick_ms)
            || !(1..=8).contains(&self.max_jobs)
            || !(100..=10000).contains(&self.shutdown_ms)
        {
            return Err(HostError::Config);
        }
        if let Some(r) = &self.token {
            r.validate()?;
        }
        if let StorageConfig::Protected { key } = &self.storage {
            key.validate()?;
        }
        let mut identities = std::collections::HashSet::new();
        for b in &self.bindings {
            b.validate().map_err(|_| HostError::Config)?;
            if self.http.protocol == foxbot_http::Protocol::BusinessV1
                && matches!(b.provider, foxbot_core::ProviderProfile::Generic { .. })
            {
                return Err(HostError::Config);
            }
            let encoded = serde_json::to_string(&b.key).map_err(|_| HostError::Config)?;
            if !identities.insert(encoded) {
                return Err(HostError::Config);
            }
        }
        Ok(())
    }
}
