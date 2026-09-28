use crate::*;
use reqwest::header::{ACCEPT, CONTENT_TYPE, RETRY_AFTER};

impl HttpError {
    pub(crate) fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Transport
                | Self::Timeout
                | Self::Status {
                    code: 408 | 429 | 500 | 502 | 503 | 504,
                    ..
                }
        )
    }
    pub(crate) fn retry_after_ms(&self) -> u64 {
        if let Self::Status {
            retry_after_ms: Some(delay),
            ..
        } = self
        {
            *delay
        } else {
            0
        }
    }
}

impl HttpReplyService {
    pub(crate) async fn post(
        &self,
        url: &str,
        body: &[u8],
        id: &str,
        attempts: u32,
        idempotent: bool,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u8>> {
        let work = async {
            let _permit = self
                .slots
                .acquire()
                .await
                .map_err(|_| HttpError::Cancelled)?;
            let mut attempt = 0;
            loop {
                attempt += 1;
                let result = self.attempt(url, body, id, idempotent).await;
                match result {
                    Err(error) if error.retryable() && idempotent && attempt < attempts => {
                        let delay = (100_u64 << (attempt - 1)).max(error.retry_after_ms());
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                    }
                    other => return other,
                }
            }
        };
        // Covers queue waiting, every response body chunk and all retry backoffs.
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(HttpError::Cancelled),
            result = tokio::time::timeout(Duration::from_millis(self.config.total_timeout_ms), work) => {
                result.unwrap_or(Err(HttpError::Timeout))
            }
        }
    }

    async fn attempt(&self, url: &str, body: &[u8], id: &str, idempotent: bool) -> Result<Vec<u8>> {
        let mut request = self
            .client
            .post(url)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_vec());
        if let Some(authorization) = &self.authorization {
            request = request.header(AUTHORIZATION, authorization);
        }
        if idempotent {
            request = request.header("Idempotency-Key", id);
        }
        let mut response = request.send().await.map_err(classify)?;
        if response.status().as_u16() != 200 {
            // The G1 contract supports delta-seconds. An unparseable/date value fails
            // closed rather than retrying before the server's requested time.
            let delay = response
                .headers()
                .get(RETRY_AFTER)
                .map(|header| {
                    header
                        .to_str()
                        .ok()
                        .and_then(|value| value.parse::<u64>().ok())
                        .and_then(|value| value.checked_mul(1000))
                        .filter(|value| *value <= 600_000)
                        .ok_or(HttpError::InvalidResponse)
                })
                .transpose()?;
            return Err(HttpError::Status {
                code: response.status().as_u16(),
                retry_after_ms: delay,
            });
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .unwrap_or_default()
            .trim();
        if content_type != "application/json" {
            return Err(HttpError::InvalidResponse);
        }
        if response
            .content_length()
            .is_some_and(|n| n > self.config.max_response_bytes as u64)
        {
            return Err(HttpError::BodyTooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(classify)? {
            if chunk.len() > self.config.max_response_bytes.saturating_sub(bytes.len()) {
                return Err(HttpError::BodyTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}
fn classify(error: reqwest::Error) -> HttpError {
    // Never retain error.source(), response bodies, endpoints or header values.
    if error.is_timeout() {
        HttpError::Timeout
    } else {
        HttpError::Transport
    }
}
