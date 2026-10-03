//! User-facing configuration: multiple endpoints, inline local keys, no Keychain setup.
//! Disk serialization is intentional. Logs, UI projections and exports are redacted.
use crate::{HostError, Result, g2d_real::write_private_json, ownership::private_directory};
use foxbot_core::ProviderProfile;
use foxbot_http::{ContextMode, HttpConfig, Protocol};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_CONFIG_BYTES: u64 = 262_144;
fn version() -> u32 {
    2
}
fn client_context() -> ContextMode {
    ContextMode::ClientManaged
}
fn default_prompt() -> String {
    "请根据最后一条用户消息清晰、简洁地回答；需要时可分段或换行，回复控制在300个汉字以内。".into()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub endpoint: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default = "client_context")]
    pub context_mode: ContextMode,
    #[serde(default)]
    pub receipt_endpoint: Option<String>,
    #[serde(default)]
    pub idempotency_supported: bool,
    #[serde(default)]
    pub staging_contract: bool,
}
impl Connection {
    pub fn http(&self) -> HttpConfig {
        HttpConfig {
            protocol: self.protocol.clone(),
            endpoint: self.endpoint.clone(),
            model: self.model.clone(),
            context_mode: self.context_mode.clone(),
            receipt_endpoint: self.receipt_endpoint.clone(),
            idempotency_supported: self.idempotency_supported,
            staging_contract: self.staging_contract,
            // The HTTP adapter only permits cleartext on numeric loopback addresses.
            allow_loopback_http: true,
            attempt_timeout_ms: 60_000,
            total_timeout_ms: 60_000,
            max_attempts: 1,
            max_response_bytes: 131_072,
            max_in_flight: 1,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if !identifier(&self.id)
            || self.name.trim().is_empty()
            || self.name.len() > 200
            || self.name.chars().any(char::is_control)
            || self.api_key.len() > 8192
            || self.api_key.chars().any(char::is_control)
            || self.api_key.trim() != self.api_key
            || self.endpoint.contains("YOUR-PROVIDER.invalid")
            || self.model.as_deref() == Some("YOUR_MODEL")
        {
            return Err(HostError::Config);
        }
        self.http().validate().map_err(|_| HostError::Config)
    }
    pub fn profile(&self, prompt: &str) -> ProviderProfile {
        match self.protocol {
            Protocol::ChatCompletions => ProviderProfile::Generic {
                system_prompt: prompt.into(),
            },
            Protocol::BusinessV1 => ProviderProfile::Custom,
        }
    }
    pub fn redacted(&self) -> Result<serde_json::Value> {
        let mut value = serde_json::to_value(self).map_err(|_| HostError::Config)?;
        value
            .as_object_mut()
            .ok_or(HostError::Config)?
            .remove("api_key");
        value["has_api_key"] = (!self.api_key.is_empty()).into();
        Ok(value)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyPreferences {
    #[serde(default = "default_prompt")]
    pub system_prompt: String,
}
impl Default for ReplyPreferences {
    fn default() -> Self {
        Self {
            system_prompt: default_prompt(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalConfig {
    #[serde(default = "version")]
    pub version: u32,
    #[serde(default)]
    pub default_connection: Option<String>,
    #[serde(default)]
    pub connections: Vec<Connection>,
    #[serde(default)]
    pub reply: ReplyPreferences,
}
impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            version: 2,
            default_connection: None,
            connections: vec![],
            reply: ReplyPreferences::default(),
        }
    }
}
impl LocalConfig {
    pub fn validate(&self) -> Result<()> {
        if self.version != 2
            || self.connections.len() > 32
            || self.reply.system_prompt.len() > 16_384
        {
            return Err(HostError::Config);
        }
        let mut ids = std::collections::HashSet::new();
        for connection in &self.connections {
            connection.validate()?;
            if !ids.insert(&connection.id) {
                return Err(HostError::Config);
            }
        }
        match &self.default_connection {
            Some(id) if !ids.contains(id) => Err(HostError::Config),
            None if !self.connections.is_empty() => Err(HostError::Config),
            _ => Ok(()),
        }
    }
    pub fn selected(&self, pinned: Option<&str>) -> Result<&Connection> {
        let id = pinned
            .or(self.default_connection.as_deref())
            .ok_or(HostError::NoConnection)?;
        // Do not fall back to another provider if a pinned connection was removed.
        self.connections
            .iter()
            .find(|c| c.id == id)
            .ok_or(HostError::NoConnection)
    }
    pub fn redacted(&self) -> Result<serde_json::Value> {
        Ok(
            serde_json::json!({"version":2, "default_connection":self.default_connection,
            "connections":self.connections.iter().map(Connection::redacted).collect::<Result<Vec<_>>>()?,
            "reply":self.reply}),
        )
    }
    pub fn export(&self) -> Result<serde_json::Value> {
        let mut value = self.redacted()?;
        for c in value["connections"]
            .as_array_mut()
            .ok_or(HostError::Config)?
        {
            c.as_object_mut()
                .ok_or(HostError::Config)?
                .remove("has_api_key");
        }
        Ok(value)
    }
}
pub fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}
pub fn default_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .ok_or(HostError::Config)?
        .join("FoxBot")
        .join("config.json"))
}
pub fn nonce() -> Result<String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|_| HostError::Storage)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(|_| HostError::Config)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_CONFIG_BYTES
    {
        return Err(HostError::Config);
    }
    let mut data = Vec::new();
    File::open(path)
        .map_err(|_| HostError::Config)?
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|_| HostError::Config)?;
    if data.len() as u64 > MAX_CONFIG_BYTES {
        return Err(HostError::Config);
    }
    Ok(data)
}
pub fn read_value(path: &Path) -> Result<serde_json::Value> {
    serde_json::from_slice(&read_bytes(path)?).map_err(|_| HostError::Config)
}
fn revision(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

struct ConfigLock(File);
impl Drop for ConfigLock {
    fn drop(&mut self) {
        // Explicit release also handles descriptors briefly inherited across concurrent fork/exec.
        let _ = self.0.unlock();
    }
}

/// One automatic per-file lock and atomic save, not a credential-management workflow.
#[derive(Clone)]
pub struct ConfigStore {
    path: PathBuf,
}
impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn lock(&self) -> Result<ConfigLock> {
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or(HostError::Config)?;
        private_directory(parent)?;
        let path = self.path.with_extension("lock");
        if let Ok(m) = fs::symlink_metadata(&path)
            && (!m.is_file() || m.file_type().is_symlink())
        {
            return Err(HostError::Config);
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path).map_err(|_| HostError::Storage)?;
        file.try_lock().map_err(|_| HostError::Busy)?;
        Ok(ConfigLock(file))
    }
    pub fn ensure(&self) -> Result<()> {
        let _lock = self.lock()?;
        match fs::symlink_metadata(&self.path) {
            Ok(_) => {
                self.load()?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                write_private_json(&self.path, &LocalConfig::default())?;
            }
            Err(_) => return Err(HostError::Storage),
        }
        Ok(())
    }
    pub fn load(&self) -> Result<(LocalConfig, String)> {
        let bytes = read_bytes(&self.path)?;
        let config: LocalConfig = serde_json::from_slice(&bytes).map_err(|_| HostError::Config)?;
        config.validate()?;
        Ok((config, revision(&bytes)))
    }
    pub fn view(&self) -> Result<serde_json::Value> {
        let (config, revision) = self.load()?;
        Ok(
            serde_json::json!({"config":config.redacted()?, "revision":revision,
            "config_path":self.path, "storage":"local_sqlite", "keychain_required":false}),
        )
    }
    pub fn edit(&self, expected_revision: &str, edit: Edit) -> Result<serde_json::Value> {
        let _lock = self.lock()?;
        let (mut config, current) = self.load()?;
        if expected_revision != current {
            return Err(HostError::ConfigChanged);
        }
        match edit {
            Edit::Save { connection } => {
                let old = config
                    .connections
                    .iter()
                    .position(|c| c.id == connection.id);
                let resolved = connection.resolve(old.map(|i| &config.connections[i]))?;
                if let Some(i) = old {
                    config.connections[i] = resolved;
                } else {
                    config.connections.push(resolved);
                }
                if config.default_connection.is_none() {
                    config.default_connection = config.connections.first().map(|c| c.id.clone());
                }
            }
            Edit::Delete { id } => {
                let i = config
                    .connections
                    .iter()
                    .position(|c| c.id == id)
                    .ok_or(HostError::NoConnection)?;
                config.connections.remove(i);
                // No silent provider substitution when deleting the default.
                if config.default_connection.as_deref() == Some(&id) {
                    if !config.connections.is_empty() {
                        return Err(HostError::DefaultConnectionInUse);
                    }
                    config.default_connection = None;
                }
            }
            Edit::Duplicate { id } => {
                let mut copy = config
                    .connections
                    .iter()
                    .find(|c| c.id == id)
                    .ok_or(HostError::NoConnection)?
                    .clone();
                copy.id = format!("connection-{}", nonce()?);
                copy.name = format!("{}（副本）", copy.name.chars().take(50).collect::<String>());
                config.connections.push(copy);
            }
            Edit::SetDefault { id } => {
                config.selected(Some(&id))?;
                config.default_connection = Some(id);
            }
            Edit::Reply { system_prompt } => {
                config.reply.system_prompt = system_prompt;
            }
        }
        config.validate()?;
        let encoded = serde_json::to_vec_pretty(&config).map_err(|_| HostError::Config)?;
        if encoded.len() as u64 + 1 > MAX_CONFIG_BYTES {
            return Err(HostError::Config);
        }
        write_private_json(&self.path, &config)?;
        self.view()
    }
    pub fn test_draft(&self, input: ConnectionInput) -> Result<Connection> {
        let (config, _) = self.load()?;
        let old = config.connections.iter().find(|c| c.id == input.id);
        input.resolve(old)
    }
}

/// Omitted api_key keeps the saved key; an explicit empty string clears it.
/// The UI never sends a masked placeholder in place of the real key.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionInput {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub endpoint: String,
    pub api_key: Option<String>,
    pub model: Option<String>,
    #[serde(default = "client_context")]
    pub context_mode: ContextMode,
    #[serde(default)]
    pub receipt_endpoint: Option<String>,
    #[serde(default)]
    pub idempotency_supported: bool,
    #[serde(default)]
    pub staging_contract: bool,
}
impl ConnectionInput {
    fn resolve(self, old: Option<&Connection>) -> Result<Connection> {
        // Changing the destination does not forward an old key to the new server implicitly.
        if self.api_key.is_none()
            && old.is_some_and(|c| c.endpoint != self.endpoint && !c.api_key.is_empty())
        {
            return Err(HostError::KeyRequiredForNewEndpoint);
        }
        let c = Connection {
            id: self.id,
            name: self.name,
            protocol: self.protocol,
            endpoint: self.endpoint,
            api_key: self
                .api_key
                .unwrap_or_else(|| old.map(|c| c.api_key.clone()).unwrap_or_default()),
            model: self.model,
            context_mode: self.context_mode,
            receipt_endpoint: self.receipt_endpoint,
            idempotency_supported: self.idempotency_supported,
            staging_contract: self.staging_contract,
        };
        c.validate()?;
        Ok(c)
    }
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    Save { connection: ConnectionInput },
    Delete { id: String },
    Duplicate { id: String },
    SetDefault { id: String },
    Reply { system_prompt: String },
}
