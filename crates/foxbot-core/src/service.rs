//! Durable provider exchanges and feedback. No networking or credentials live here.
//! Receipt replay is independent of chat dispatch and cannot call MessageChannel::send.
use crate::*;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

pub const MAX_RECEIPT_ATTEMPTS: u32 = 8;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceReceipt {
    pub schema_version: String,
    pub receipt_id: String,
    pub request_id: String,
    pub conversation_ref: String,
    /// UNKNOWN is revision 1; a final disposition is revision 2. Servers ignore older revisions.
    pub revision: u32,
    pub disposition: ServiceDisposition,
    pub action_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceDisposition {
    ObservedOutgoing,
    Unknown,
    Cancelled,
    NoReply,
    Handoff,
}

#[derive(Clone, Debug)]
pub struct ReceiptClaim {
    pub receipt: ServiceReceipt,
    pub attempt: u32,
    profile_tag: String,
}
impl ReceiptClaim {
    pub fn profile_tag(&self) -> &str {
        &self.profile_tag
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ServiceQueueSummary {
    pub pending: u64,
    pub in_flight: u64,
    pub acked: u64,
    pub suspended: u64,
}

impl Runtime {
    /// Stateful history must settle before a later turn. Also blocks changing profiles
    /// to silently escape an unresolved earlier service-managed turn.
    pub fn service_ready(&self, key: &ConversationKey) -> Result<bool> {
        let pending: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM service_exchanges e JOIN tasks t ON t.id=e.request_id
             WHERE t.conversation=?1 AND e.stateful=1 AND NOT EXISTS(
               SELECT 1 FROM service_receipts r WHERE r.request_id=e.request_id
               AND r.revision=2 AND r.status='ACKED'))",
            [key.encoded()?],
            |row| row.get(0),
        )?;
        Ok(!pending)
    }

    /// Called before any HTTP request. Only a one-way config fingerprint is stored;
    /// URLs, credentials and the network client are not persisted in this table.
    pub fn track_service_exchange(
        &mut self,
        request_id: &str,
        profile_tag: &str,
        conversation_ref: &str,
        stateful: bool,
        wants_receipts: bool,
    ) -> Result<()> {
        crate::model::valid_id(request_id)?;
        crate::model::valid_id(conversation_ref)?;
        if profile_tag.len() != 64
            || !profile_tag.bytes().all(|b| b.is_ascii_hexdigit())
            || (stateful && !wants_receipts)
        {
            return Err(Error::Invalid("provider exchange contract"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: Option<String> = tx
            .query_row("SELECT state FROM tasks WHERE id=?1", [request_id], |r| {
                r.get(0)
            })
            .optional()?;
        if state.as_deref() != Some("GENERATING") {
            return Err(Error::Stale);
        }
        tx.execute(
            "INSERT INTO service_exchanges VALUES (?1,?2,?3,?4,?5)",
            params![
                request_id,
                profile_tag,
                conversation_ref,
                stateful,
                wants_receipts
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Verify the captured configuration before accepting a network completion.
    pub fn check_service_exchange(&self, request_id: &str, profile_tag: &str) -> Result<()> {
        let found: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM service_exchanges WHERE request_id=?1 AND profile_tag=?2)",
            params![request_id, profile_tag],
            |r| r.get(0),
        )?;
        if !found {
            return Err(Error::Stale);
        }
        Ok(())
    }

    /// Reconstruct missing feedback from the durable task/outbox facts. This closes
    /// a crash gap even when the process died before enqueuing feedback. No HTTP here.
    pub fn sync_service_receipts(&mut self) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rows = {
            let mut stmt = tx.prepare(
                "SELECT e.request_id,e.conversation_ref,COALESCE(o.state,t.state),o.id
                 FROM service_exchanges e JOIN tasks t ON t.id=e.request_id
                 LEFT JOIN outbox o ON o.task_id=t.id WHERE e.wants_receipts=1",
            )?;
            stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (request_id, conversation_ref, state, action_id) in rows {
            let (revision, disposition) = match state.as_str() {
                "VERIFIED_OUTGOING" => (2, ServiceDisposition::ObservedOutgoing),
                "EXECUTING" | "SUBMITTED" | "UNKNOWN" => (1, ServiceDisposition::Unknown),
                "STALE" | "ERROR" | "BLOCKED" => (2, ServiceDisposition::Cancelled),
                "NO_REPLY" => (2, ServiceDisposition::NoReply),
                "HANDOFF" => (2, ServiceDisposition::Handoff),
                _ => continue,
            };
            let receipt = ServiceReceipt {
                schema_version: "0.1".into(),
                receipt_id: format!("{request_id}:{revision}"),
                request_id,
                conversation_ref,
                revision,
                disposition,
                action_id,
            };
            let payload = serde_json::to_string(&receipt)?;
            let old: Option<String> = tx
                .query_row(
                    "SELECT payload FROM service_receipts WHERE id=?1",
                    [&receipt.receipt_id],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(old) = old {
                if old != payload {
                    return Err(Error::Schema);
                }
            } else {
                tx.execute(
                    "INSERT INTO service_receipts(id,request_id,revision,payload,status)
                    VALUES (?1,?2,?3,?4,'PENDING')",
                    params![
                        receipt.receipt_id,
                        receipt.request_id,
                        receipt.revision,
                        payload
                    ],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Persist an attempt before network I/O. A process crash resets IN_FLIGHT to
    /// PENDING on open; the same immutable receipt id is retained for server dedupe.
    pub fn claim_service_receipt(
        &mut self,
        profile_tag: &str,
        now_ms: u64,
    ) -> Result<Option<ReceiptClaim>> {
        let now = i64::try_from(now_ms).map_err(|_| Error::Invalid("clock range"))?;
        self.sync_service_receipts()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("UPDATE service_receipts SET status='SUSPENDED' WHERE status='PENDING' AND attempts>=?1",
            [MAX_RECEIPT_ATTEMPTS])?;
        let row: Option<(String, String, u32)> = tx
            .query_row(
                "SELECT r.id,r.payload,r.attempts FROM service_receipts r
             JOIN service_exchanges e ON e.request_id=r.request_id
             WHERE e.profile_tag=?1 AND r.status='PENDING' AND r.next_ms<=?2
             ORDER BY r.rowid LIMIT 1",
                params![profile_tag, now],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((id, payload, attempts)) = row else {
            tx.commit()?;
            return Ok(None);
        };
        let receipt: ServiceReceipt = serde_json::from_str(&payload)?;
        if receipt.receipt_id != id {
            return Err(Error::Schema);
        }
        let attempt = attempts + 1;
        tx.execute(
            "UPDATE service_receipts SET status='IN_FLIGHT',attempts=?2 WHERE id=?1",
            params![id, attempt],
        )?;
        tx.commit()?;
        Ok(Some(ReceiptClaim {
            receipt,
            attempt,
            profile_tag: profile_tag.to_owned(),
        }))
    }

    pub fn acknowledge_service_receipt(&mut self, claim: &ReceiptClaim) -> Result<()> {
        let count = self.conn.execute(
            "UPDATE service_receipts SET status='ACKED' WHERE id=?1 AND attempts=?2 AND status='IN_FLIGHT'",
            params![claim.receipt.receipt_id,claim.attempt])?;
        if count != 1 {
            return Err(Error::Stale);
        }
        Ok(())
    }

    pub fn defer_service_receipt(
        &mut self,
        claim: &ReceiptClaim,
        now_ms: u64,
        retryable: bool,
        minimum_delay_ms: u64,
    ) -> Result<()> {
        let delay = (250_u64 << claim.attempt.saturating_sub(1).min(7)).max(minimum_delay_ms);
        let next = i64::try_from(now_ms.saturating_add(delay))
            .map_err(|_| Error::Invalid("clock range"))?;
        let status = if retryable && claim.attempt < MAX_RECEIPT_ATTEMPTS {
            "PENDING"
        } else {
            "SUSPENDED"
        };
        let count = self.conn.execute(
            "UPDATE service_receipts SET status=?3,next_ms=?4 WHERE id=?1 AND attempts=?2 AND status='IN_FLIGHT'",
            params![claim.receipt.receipt_id,claim.attempt,status,next])?;
        if count != 1 {
            return Err(Error::Stale);
        }
        Ok(())
    }

    pub fn service_queue_summary(&self) -> Result<ServiceQueueSummary> {
        let count = |state: &str| -> Result<u64> {
            let value: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM service_receipts WHERE status=?1",
                [state],
                |r| r.get(0),
            )?;
            u64::try_from(value).map_err(|_| Error::Schema)
        };
        Ok(ServiceQueueSummary {
            pending: count("PENDING")?,
            in_flight: count("IN_FLIGHT")?,
            acked: count("ACKED")?,
            suspended: count("SUSPENDED")?,
        })
    }
}
