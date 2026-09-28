use crate::{HttpError, Result};
use reqwest::Url;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    ChatCompletions,
    BusinessV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    ClientManaged,
    ServiceManaged,
}

/// Non-secret configuration. Exact endpoints; no implicit provider/model fallback.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpConfig {
    pub protocol: Protocol,
    pub endpoint: String,
    pub model: Option<String>,
    pub context_mode: ContextMode,
    pub receipt_endpoint: Option<String>,
    pub idempotency_supported: bool,
    pub staging_contract: bool,
    pub allow_loopback_http: bool,
    pub attempt_timeout_ms: u64,
    pub total_timeout_ms: u64,
    pub max_attempts: u32,
    pub max_response_bytes: usize,
    pub max_in_flight: usize,
}

impl HttpConfig {
    pub fn validate(&self) -> Result<()> {
        let endpoint = self.url(&self.endpoint)?;
        if let Some(receipt) = &self.receipt_endpoint {
            let receipt = self.url(receipt)?;
            if endpoint.origin() != receipt.origin() {
                return Err(HttpError::Config);
            }
        }
        if self.attempt_timeout_ms == 0
            || self.total_timeout_ms < self.attempt_timeout_ms
            || self.total_timeout_ms > 600_000
            || self.max_attempts == 0
            || self.max_attempts > 3
            || self.max_response_bytes < 128
            || self.max_response_bytes > 1_048_576
            || self.max_in_flight == 0
            || self.max_in_flight > 16
            || (self.max_attempts > 1 && !self.idempotency_supported)
        {
            return Err(HttpError::Config);
        }
        match self.protocol {
            Protocol::ChatCompletions => {
                if self.context_mode != ContextMode::ClientManaged
                    || self.receipt_endpoint.is_some()
                    || self.staging_contract
                    || self
                        .model
                        .as_ref()
                        .is_none_or(|m| m.trim().is_empty() || m.len() > 256)
                {
                    return Err(HttpError::Config);
                }
            }
            Protocol::BusinessV1 => {
                if self.model.is_some() {
                    return Err(HttpError::Config);
                }
                if self.context_mode == ContextMode::ServiceManaged
                    && (!self.idempotency_supported
                        || !self.staging_contract
                        || self.receipt_endpoint.is_none())
                {
                    return Err(HttpError::Config);
                }
                if self.receipt_endpoint.is_some()
                    && (!self.idempotency_supported || !self.staging_contract)
                {
                    return Err(HttpError::Config);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn url(&self, value: &str) -> Result<Url> {
        if value.len() > 2048 {
            return Err(HttpError::Config);
        }
        let url = Url::parse(value).map_err(|_| HttpError::Config)?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.host_str().is_none()
        {
            return Err(HttpError::Config);
        }
        let loopback = url
            .host_str()
            .and_then(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
            .is_some_and(|ip| ip.is_loopback());
        if url.scheme() != "https"
            && !(url.scheme() == "http" && loopback && self.allow_loopback_http)
        {
            return Err(HttpError::Config);
        }
        Ok(url)
    }
}
