use crate::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
    time::Duration,
};
use uuid::Uuid;

/// One owner per canonical simulation state directory, for the entire runtime lifetime.
/// This does not fence a second device or a different state directory.
pub struct Runtime {
    pub(crate) conn: Connection,
    _owner: File,
    encrypted: bool,
}

struct Session {
    binding: Binding,
    revision: u64,
    public_ref: String,
    attempts: u32,
}

struct Task {
    conversation: String,
    request: ReplyRequest,
    response: Option<String>,
    created_ms: u64,
    state: String,
}

impl Runtime {
    /// Normal local storage for real usage. No credential setup and no implicit conversion
    /// of existing encrypted databases. Runtime/outbox/recovery semantics are unchanged.
    pub fn open_local(directory: impl AsRef<Path>) -> Result<Self> {
        let database = directory.as_ref().join("ledger.sqlite3");
        if let Ok(metadata) = fs::symlink_metadata(&database) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(Error::UnsafeState);
            }
            if metadata.len() > 0 {
                let mut header = [0u8; 16];
                let mut file = fs::File::open(&database)?;
                std::io::Read::read_exact(&mut file, &mut header)
                    .map_err(|_| Error::ProtectedStore)?;
                if &header != b"SQLite format 3\0" {
                    return Err(Error::ProtectedStore);
                }
            }
        }
        Self::open_inner(directory.as_ref(), None)
    }

    /// Explicit plaintext fixture entry. Never used as a fallback for a protected store.
    pub fn open_simulation(directory: impl AsRef<Path>) -> Result<Self> {
        Self::open_inner(directory.as_ref(), None)
    }

    /// The caller obtains a random 32-byte key from its credential store. No implicit
    /// key creation, rekeying, plaintext import or key-loss recovery happens here.
    pub fn open_encrypted(directory: impl AsRef<Path>, key: &[u8; 32]) -> Result<Self> {
        if !cfg!(feature = "encrypted-ledger") {
            return Err(Error::ProtectedStore);
        }
        Self::open_inner(directory.as_ref(), Some(key))
    }

    fn open_inner(directory: &Path, key: Option<&[u8; 32]>) -> Result<Self> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(directory)?;
        let metadata = fs::symlink_metadata(directory)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(Error::UnsafeState);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(Error::UnsafeState);
            }
        }
        let directory = fs::canonicalize(directory)?;
        let owner = private_file(&directory.join("owner.lock"))?;
        owner.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => Error::Busy,
            fs::TryLockError::Error(error) => Error::Io(error),
        })?;
        // Never delete the lock file: replacing its inode could admit another owner.
        let database = directory.join("ledger.sqlite3");
        let was_empty = private_file(&database)?.metadata()?.len() == 0;
        for name in [
            "ledger.sqlite3-wal",
            "ledger.sqlite3-shm",
            "ledger.sqlite3-journal",
        ] {
            if let Ok(meta) = fs::symlink_metadata(directory.join(name))
                && (!meta.is_file() || meta.file_type().is_symlink())
            {
                return Err(Error::UnsafeState);
            }
        }
        let mut conn = Connection::open(database)?;
        if let Some(key) = key {
            use std::fmt::Write;
            let mut raw = zeroize::Zeroizing::new(String::from("x'"));
            for byte in key {
                write!(&mut *raw, "{byte:02x}").map_err(|_| Error::ProtectedStore)?;
            }
            raw.push('\'');
            conn.pragma_update(None, "key", raw.as_str())
                .map_err(|_| Error::ProtectedStore)?;
            let cipher: String = conn
                .query_row("PRAGMA cipher_version", [], |r| r.get(0))
                .map_err(|_| Error::ProtectedStore)?;
            if cipher.is_empty() {
                return Err(Error::ProtectedStore);
            }
            // PRAGMA key alone does not test the key. Read before ANY schema mutation.
            conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(|_| Error::ProtectedStore)?;
            conn.execute_batch("PRAGMA cipher_memory_security=ON; PRAGMA temp_store=MEMORY;")
                .map_err(|_| Error::ProtectedStore)?;
        }
        conn.busy_timeout(Duration::from_secs(5))?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let application: i64 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        if !((was_empty && version == 0 && application == 0)
            || ((1..=3).contains(&version) && application == 0x46425831))
        {
            return Err(Error::Schema);
        }
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
        )?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(include_str!("schema.sql"))?;
        // A lost HTTP ACK is replayed with the same immutable receipt id, never by resending chat.
        tx.execute(
            "UPDATE service_receipts SET status='PENDING' WHERE status='IN_FLIGHT'",
            [],
        )?;
        // Recovery only after acquiring the lifetime owner lock. Absence of a receipt
        // is NOT evidence that an external side effect did not happen.
        tx.execute("INSERT INTO transitions(action_id,state,reason)
            SELECT id,'UNKNOWN','process_recovery' FROM outbox WHERE state IN ('EXECUTING','SUBMITTED')", [])?;
        tx.execute(
            "UPDATE outbox SET state='UNKNOWN',reason='process_recovery'
            WHERE state IN ('EXECUTING','SUBMITTED')",
            [],
        )?;
        tx.execute(
            "UPDATE tasks SET state='UNKNOWN' WHERE id IN
            (SELECT task_id FROM outbox WHERE state='UNKNOWN')",
            [],
        )?;
        tx.execute(
            "UPDATE tasks SET state='ERROR' WHERE state='GENERATING'",
            [],
        )?;
        tx.execute(
            "UPDATE messages SET disposition='HANDLED' WHERE task_id IN
            (SELECT id FROM tasks WHERE state='ERROR')",
            [],
        )?;
        tx.commit()?;
        Ok(Self {
            conn,
            _owner: owner,
            encrypted: key.is_some(),
        })
    }

    pub fn bind(&mut self, binding: &Binding) -> Result<()> {
        binding.validate()?;
        let key = binding.key.encoded()?;
        let payload = serde_json::to_string(binding)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<String> = tx
            .query_row("SELECT binding FROM sessions WHERE key=?1", [&key], |r| {
                r.get(0)
            })
            .optional()?;
        match old {
            None => {
                tx.execute(
                    "INSERT INTO sessions(key,public_ref,binding) VALUES (?1,?2,?3)",
                    params![key, id(), payload],
                )?;
            }
            Some(old) if old != payload => {
                invalidate(&tx, &key, false)?;
                let previous: Binding = serde_json::from_str(&old)?;
                let new_scope = previous.identity_epoch != binding.identity_epoch
                    || previous.profile_version != binding.profile_version
                    || previous.provider != binding.provider;
                tx.execute(
                    "UPDATE sessions SET binding=?2,revision=revision+1,
                    public_ref=CASE WHEN ?3 THEN ?4 ELSE public_ref END WHERE key=?1",
                    params![key, payload, new_scope, id()],
                )?;
                // Old queued input must not be replayed after an authorization/configuration change.
                tx.execute("UPDATE messages SET disposition='IGNORED' WHERE conversation=?1 AND disposition='QUEUED'", [&key])?;
            }
            Some(_) => {}
        }
        tx.commit()?;
        Ok(())
    }

    /// Host suspension is independent of each conversation's authorization/handoff state.
    pub fn host_is_paused(&self) -> Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT paused FROM host_control WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?
            .unwrap_or(false))
    }

    pub fn set_host_paused(&mut self, paused: bool) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO host_control(id,paused) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET paused=excluded.paused", [paused])?;
        if paused {
            let keys: Vec<String> = {
                let mut stmt = tx.prepare("SELECT key FROM sessions")?;
                stmt.query_map([], |r| r.get(0))?
                    .collect::<std::result::Result<_, _>>()?
            };
            for key in keys {
                invalidate(&tx, &key, false)?;
            }
            tx.execute("UPDATE sessions SET revision=revision+1", [])?;
            tx.execute(
                "UPDATE messages SET disposition='IGNORED' WHERE disposition='QUEUED'",
                [],
            )?;
        }
        tx.commit()?;
        self.sync_service_receipts()
    }

    pub fn is_encrypted(&self) -> bool {
        self.encrypted
    }

    /// Bounded counters for continuous status; does not enumerate historical actions.
    pub fn host_counts(&self) -> Result<(u64, u64, u64, u64)> {
        let count =
            |sql: &str| -> Result<u64> { unsigned(self.conn.query_row(sql, [], |r| r.get(0))?) };
        Ok((
            count("SELECT COUNT(*) FROM messages")?,
            count("SELECT COUNT(*) FROM tasks")?,
            count("SELECT COUNT(*) FROM tasks WHERE state='READY'")?,
            count(
                "SELECT COUNT(*) FROM outbox WHERE state IN ('EXECUTING','SUBMITTED','UNKNOWN')",
            )?,
        ))
    }

    pub fn binding(&self, key: &ConversationKey) -> Result<Option<Binding>> {
        let payload: Option<String> = self
            .conn
            .query_row(
                "SELECT binding FROM sessions WHERE key=?1",
                [key.encoded()?],
                |r| r.get(0),
            )
            .optional()?;
        payload
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }

    pub fn ready_requests(&self, key: &ConversationKey) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT id FROM tasks WHERE conversation=?1 AND state='READY' ORDER BY created_ms,rowid LIMIT 16")?;
        Ok(stmt
            .query_map([key.encoded()?], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?)
    }

    pub fn pause(&mut self, key: &ConversationKey) -> Result<()> {
        let mut binding = session(&self.conn, &key.encoded()?)?.binding;
        binding.enabled = false;
        self.bind(&binding)
    }

    pub fn ingest(&mut self, observation: &Observation) -> Result<IngestOutcome> {
        let host_paused = self.host_is_paused()?;
        let key = observation.key.encoded()?;
        crate::model::valid_id(&observation.source_event_id)?;
        let message = &observation.message;
        if let Some(value) = &message.canonical_id {
            crate::model::valid_id(value)?;
        }
        if let Some(value) = &message.sender {
            crate::model::valid_id(value)?;
        }
        if let Some(value) = &message.reply_to {
            crate::model::valid_id(value)?;
        }
        if message.text.as_ref().is_some_and(|v| v.len() > 16_384)
            || (message.kind == ContentKind::Text
                && message.complete
                && message.text.as_ref().is_none_or(|v| v.trim().is_empty()))
        {
            return Err(Error::Invalid("message content"));
        }
        let timestamp = signed(observation.observed_ms)?;
        let payload = serde_json::to_string(message)?;
        // Scope capture IDs to the identity epoch; a relogin may reuse source IDs.
        let source = serde_json::to_string(&(observation.identity_epoch, observation.source))?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let config = session(&tx, &key)?.binding;
        if observation.identity_epoch != config.identity_epoch {
            return Err(Error::Stale);
        }
        let prior: Option<(String,String)> = tx.query_row(
            "SELECT message_id,payload FROM observations WHERE conversation=?1 AND source=?2 AND source_id=?3",
            params![key, source, observation.source_event_id], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((message_id, old)) = prior {
            if old == payload {
                return Ok(IngestOutcome::Duplicate);
            }
            quarantine(&tx, &key, &message_id)?;
            tx.commit()?;
            return Ok(IngestOutcome::Ambiguous);
        }
        if let Some(canonical) = &message.canonical_id {
            let prior: Option<(String, String)> = tx
                .query_row(
                    "SELECT id,payload FROM messages WHERE conversation=?1 AND canonical_id=?2 AND identity_epoch=?3",
                    params![key, canonical, signed(observation.identity_epoch)?],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((message_id, old)) = prior {
                tx.execute(
                    "INSERT INTO observations VALUES (?1,?2,?3,?4,?5)",
                    params![
                        key,
                        source,
                        observation.source_event_id,
                        message_id,
                        payload
                    ],
                )?;
                let outcome = if old == payload {
                    IngestOutcome::Duplicate
                } else {
                    quarantine(&tx, &key, &message_id)?;
                    IngestOutcome::Ambiguous
                };
                tx.commit()?;
                return Ok(outcome);
            }
        }
        let ambiguous = message.canonical_id.is_none()
            || message.direction == Direction::Unknown
            || (message.direction == Direction::Incoming && message.sender.is_none());
        let eligible = !ambiguous
            && !host_paused
            && config.enabled
            && message.direction == Direction::Incoming
            && (config.kind == ConversationKind::Private
                || config.group_all_messages
                || message.mention == Mention::Verified);
        let (disposition, outcome) = if ambiguous {
            ("AMBIGUOUS", IngestOutcome::Ambiguous)
        } else if observation.historical {
            ("BASELINED", IngestOutcome::Baseline)
        } else if eligible {
            ("QUEUED", IngestOutcome::Queued)
        } else {
            ("IGNORED", IngestOutcome::Ignored)
        };
        if eligible && !observation.historical {
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM messages WHERE conversation=?1
                AND disposition IN ('QUEUED','ASSIGNED')",
                [&key],
                |r| r.get(0),
            )?;
            if count >= config.max_pending as i64 {
                return Err(Error::Backpressure);
            }
        }
        let message_id = id();
        tx.execute(
            "INSERT INTO messages(id,conversation,canonical_id,payload,observed_ms,disposition,identity_epoch)
            VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                message_id,
                key,
                message.canonical_id,
                payload,
                timestamp,
                disposition,
                signed(observation.identity_epoch)?
            ],
        )?;
        tx.execute(
            "INSERT INTO observations VALUES (?1,?2,?3,?4,?5)",
            params![
                key,
                source,
                observation.source_event_id,
                message_id,
                payload
            ],
        )?;
        if !observation.historical
            && (matches!(message.direction, Direction::Incoming | Direction::Own) || ambiguous)
        {
            invalidate(&tx, &key, message.direction != Direction::Own)?;
            if message.direction == Direction::Own {
                tx.execute("UPDATE messages SET disposition='IGNORED' WHERE conversation=?1 AND disposition='QUEUED'", [&key])?;
            }
            tx.execute(
                "UPDATE sessions SET revision=revision+1 WHERE key=?1",
                [&key],
            )?;
        }
        tx.commit()?;
        Ok(outcome)
    }

    /// Deterministic scheduler tick. Caller supplies a monotonically advancing time
    /// within a run; rollback makes a task ineligible, never immediately overdue.
    pub fn begin_reply(
        &mut self,
        key: &ConversationKey,
        now_ms: u64,
        user_request: Option<&str>,
    ) -> Result<Option<ReplyRequest>> {
        signed(now_ms)?;
        if self.host_is_paused()? {
            return Ok(None);
        }
        if user_request.is_some_and(|v| v.len() > 16_384) {
            return Err(Error::Invalid("user request size"));
        }
        let key = key.encoded()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state = session(&tx, &key)?;
        if !state.binding.enabled
            || (state.binding.mode == Mode::Assisted && user_request.is_none())
        {
            return Ok(None);
        }
        let active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE conversation=?1
            AND state IN ('GENERATING','READY','PREPARED','EXECUTING','SUBMITTED','UNKNOWN'))",
            [&key],
            |r| r.get(0),
        )?;
        if active {
            return Ok(None);
        }
        let timing: (Option<i64>,Option<i64>) = tx.query_row(
            "SELECT MIN(observed_ms),MAX(observed_ms) FROM messages WHERE conversation=?1 AND disposition='QUEUED'",
            [&key], |r| Ok((r.get(0)?,r.get(1)?)))?;
        let (Some(first), Some(last)) = timing else {
            return Ok(None);
        };
        let (first, last) = (unsigned(first)?, unsigned(last)?);
        if now_ms < last {
            return Ok(None);
        }
        let due = first
            .saturating_add(state.binding.max_wait_ms)
            .min(last.saturating_add(state.binding.quiet_ms));
        if user_request.is_none() && now_ms < due {
            return Ok(None);
        }
        let events = read_events(&tx, &key, true, state.binding.max_batch, None)?;
        let context = read_events(
            &tx,
            &key,
            false,
            32,
            events.last().map(|e| e.event_id.as_str()),
        )?;
        let request = ReplyRequest {
            request_id: id(),
            conversation_ref: state.public_ref,
            session_revision: state.revision,
            provider_profile_version: state.binding.profile_version,
            context_complete: context.iter().all(|e| e.message.complete),
            input_events: events,
            context,
            system_prompt: match &state.binding.provider {
                ProviderProfile::Custom => None,
                ProviderProfile::Generic { system_prompt } => Some(system_prompt.clone()),
            },
            user_request: user_request.map(str::to_owned),
        };
        tx.execute(
            "INSERT INTO tasks(id,conversation,revision,profile_version,request,created_ms,state)
            VALUES (?1,?2,?3,?4,?5,?6,'GENERATING')",
            params![
                request.request_id,
                key,
                signed(state.revision)?,
                signed(state.binding.profile_version)?,
                serde_json::to_string(&request)?,
                signed(now_ms)?
            ],
        )?;
        for event in &request.input_events {
            tx.execute("UPDATE messages SET disposition='ASSIGNED',task_id=?2 WHERE id=?1 AND disposition='QUEUED'",
                params![event.event_id,request.request_id])?;
        }
        tx.commit()?;
        Ok(Some(request))
    }

    pub fn accept_json(&mut self, request_id: &str, json: &str, now_ms: u64) -> Result<()> {
        if json.len() > 131_072 {
            self.fail_reply(request_id)?;
            return Err(Error::Invalid("response size"));
        }
        match serde_json::from_str::<ReplyResponse>(json) {
            Ok(response) => self.accept_reply(request_id, &response, now_ms),
            Err(_) => {
                self.fail_reply(request_id)?;
                Err(Error::Invalid("response schema"))
            }
        }
    }

    pub fn accept_reply(
        &mut self,
        request_id: &str,
        response: &ReplyResponse,
        now_ms: u64,
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task = task(&tx, request_id)?;
        let current = session(&tx, &task.conversation)?;
        if task.state != "GENERATING" {
            return Err(Error::Stale);
        }
        if !is_current(&task, &current, now_ms) {
            finish_task(&tx, request_id, "STALE")?;
            tx.commit()?;
            return Err(Error::Stale);
        }
        let expected: Vec<_> = task
            .request
            .input_events
            .iter()
            .map(|e| e.event_id.clone())
            .collect();
        let content_ok = match &response.outcome {
            ReplyOutcome::Reply { text } => valid_reply(text, current.binding.max_reply_chars),
            ReplyOutcome::Handoff { reason } => !reason.trim().is_empty() && reason.len() <= 4096,
            ReplyOutcome::NoReply => true,
        };
        if response.request_id != request_id
            || response.in_reply_to != expected
            || !response.complete
            || !content_ok
        {
            finish_task(&tx, request_id, "ERROR")?;
            tx.commit()?;
            return Err(Error::Invalid("response binding or content"));
        }
        let next = match response.outcome {
            ReplyOutcome::Reply { .. } => "READY",
            ReplyOutcome::NoReply => "NO_REPLY",
            ReplyOutcome::Handoff { .. } => "HANDOFF",
        };
        tx.execute(
            "UPDATE tasks SET state=?2,response=?3 WHERE id=?1",
            params![request_id, next, serde_json::to_string(response)?],
        )?;
        if next != "READY" {
            finish_task(&tx, request_id, next)?;
        }
        if next == "HANDOFF" {
            let mut binding = current.binding;
            binding.enabled = false;
            tx.execute(
                "UPDATE sessions SET binding=?2,revision=revision+1 WHERE key=?1",
                params![task.conversation, serde_json::to_string(&binding)?],
            )?;
            tx.execute("UPDATE messages SET disposition='IGNORED' WHERE conversation=?1 AND disposition='QUEUED'", [&task.conversation])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn fail_reply(&mut self, request_id: &str) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if task(&tx, request_id)?.state != "GENERATING" {
            return Err(Error::Stale);
        }
        finish_task(&tx, request_id, "ERROR")?;
        tx.commit()?;
        Ok(())
    }

    pub fn generate_once<P: ReplyProvider>(
        &mut self,
        key: &ConversationKey,
        now_ms: u64,
        user_request: Option<&str>,
        provider: &mut P,
    ) -> Result<Option<String>> {
        let Some(request) = self.begin_reply(key, now_ms, user_request)? else {
            return Ok(None);
        };
        match provider.generate(&request) {
            Ok(response) => self.accept_reply(&request.request_id, &response, now_ms)?,
            Err(error) => {
                self.fail_reply(&request.request_id)?;
                return Err(error);
            }
        }
        Ok(Some(request.request_id))
    }

    pub fn prepare_send(
        &mut self,
        request_id: &str,
        now_ms: u64,
        approved_by_user: bool,
    ) -> Result<String> {
        if self.host_is_paused()? {
            return Err(Error::Blocked("host paused"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task = task(&tx, request_id)?;
        let current = session(&tx, &task.conversation)?;
        if task.state != "READY" {
            return Err(Error::Stale);
        }
        if !is_current(&task, &current, now_ms) {
            finish_task(&tx, request_id, "STALE")?;
            tx.commit()?;
            return Err(Error::Stale);
        }
        if current.binding.mode != Mode::AutoReply && !approved_by_user {
            return Err(Error::Blocked("approval required"));
        }
        let response: ReplyResponse =
            serde_json::from_str(task.response.as_deref().ok_or(Error::Schema)?)?;
        let ReplyOutcome::Reply { text } = response.outcome else {
            return Err(Error::Stale);
        };
        let action = OutboundAction {
            action_id: id(),
            request_id: request_id.to_owned(),
            target: current.binding.key,
            identity_epoch: current.binding.identity_epoch,
            session_revision: current.revision,
            profile_version: current.binding.profile_version,
            text,
            approved_by_user,
            created_ms: now_ms,
        };
        tx.execute("INSERT INTO outbox(id,task_id,conversation,payload,state) VALUES (?1,?2,?3,?4,'PREPARED')",
            params![action.action_id,request_id,task.conversation,serde_json::to_string(&action)?])?;
        transition(&tx, &action.action_id, ActionState::Prepared, "prepared")?;
        tx.commit()?;
        Ok(action.action_id)
    }

    pub fn action(&self, action_id: &str) -> Result<(OutboundAction, ActionState)> {
        let (payload, state, task_id, conversation): (String, String, String, String) = self
            .conn
            .query_row(
                "SELECT payload,state,task_id,conversation FROM outbox WHERE id=?1",
                [action_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        let action: OutboundAction = serde_json::from_str(&payload)?;
        if action.action_id != action_id
            || action.request_id != task_id
            || action.target.encoded()? != conversation
        {
            return Err(Error::Schema);
        }
        Ok((action, ActionState::parse(&state)?))
    }

    pub fn preview_before_fill_gate(
        &self,
        action_id: &str,
        now_ms: u64,
        live: &LiveTarget,
    ) -> Result<SendGateReport> {
        self.evaluate_send_gate(
            action_id,
            now_ms,
            live,
            SendGatePhase::BeforeFill,
            None,
            &[ActionState::Prepared],
        )
    }

    pub fn preview_before_send_gate(
        &self,
        action_id: &str,
        now_ms: u64,
        before: &LiveTarget,
        after: &LiveTarget,
    ) -> Result<SendGateReport> {
        self.evaluate_send_gate(
            action_id,
            now_ms,
            after,
            SendGatePhase::BeforeSend,
            Some(before),
            &[ActionState::Prepared],
        )
    }

    fn persistent_send_blockers(
        &self,
        action: &OutboundAction,
        state: ActionState,
        now_ms: u64,
        allowed_states: &[ActionState],
    ) -> Result<Vec<SendGateBlocker>> {
        let task = task(&self.conn, &action.request_id)?;
        let current = session(&self.conn, &task.conversation)?;
        let mut blockers = Vec::new();
        if self.host_is_paused()? {
            blockers.push(SendGateBlocker::HostPaused);
        }
        if !allowed_states.contains(&state) {
            blockers.push(SendGateBlocker::ActionState);
        }
        if !is_current(&task, &current, now_ms)
            || action.target != current.binding.key
            || action.identity_epoch != current.binding.identity_epoch
            || action.session_revision != current.revision
            || action.profile_version != current.binding.profile_version
            || (current.binding.mode != Mode::AutoReply && !action.approved_by_user)
        {
            blockers.push(SendGateBlocker::StaleAction);
        }
        // The attempt budget is consumed exactly once when PREPARED crosses into
        // EXECUTING. Rechecking it after that transition would reject the very
        // attempt that was just authorized.
        if state != ActionState::Executing && current.attempts >= current.binding.max_auto_sends {
            blockers.push(SendGateBlocker::AttemptBudget);
        }
        let unresolved: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM outbox WHERE conversation=?1 AND id<>?2
             AND state IN ('EXECUTING','SUBMITTED','UNKNOWN'))",
            params![task.conversation, action.action_id],
            |row| row.get(0),
        )?;
        if unresolved {
            blockers.push(SendGateBlocker::UnresolvedPriorSend);
        }
        Ok(blockers)
    }

    fn evaluate_send_gate(
        &self,
        action_id: &str,
        now_ms: u64,
        live: &LiveTarget,
        phase: SendGatePhase,
        before: Option<&LiveTarget>,
        allowed_states: &[ActionState],
    ) -> Result<SendGateReport> {
        let (action, state) = self.action(action_id)?;
        let mut blockers = self.persistent_send_blockers(&action, state, now_ms, allowed_states)?;

        if live.key != action.target {
            blockers.push(SendGateBlocker::TargetMismatch);
        }
        if live.identity_epoch != action.identity_epoch {
            blockers.push(SendGateBlocker::IdentityEpochMismatch);
        }
        if live.application_session_ref.is_empty() {
            blockers.push(SendGateBlocker::ApplicationSessionMissing);
        }
        if live.conversation_surface_ref.is_empty() {
            blockers.push(SendGateBlocker::ConversationSurfaceMissing);
        }
        if live.window_ref.is_empty() || live.editor_ref.is_empty() {
            blockers.push(SendGateBlocker::SurfaceMissing);
        }
        if !live.frontmost {
            blockers.push(SendGateBlocker::NotFrontmost);
        }
        if live.conversation_changed {
            blockers.push(SendGateBlocker::ConversationChanged);
        }
        if !live.permitted {
            blockers.push(SendGateBlocker::NotPermitted);
        }

        match phase {
            SendGatePhase::BeforeFill => {
                if live.draft != Draft::Empty {
                    blockers.push(SendGateBlocker::DraftNotEmpty);
                }
            }
            SendGatePhase::BeforeSend => {
                let Some(before) = before else {
                    blockers.push(SendGateBlocker::SurfaceChanged);
                    return Ok(SendGateReport {
                        phase,
                        allowed: false,
                        blockers,
                    });
                };
                if !before.same_surface(live) {
                    blockers.push(SendGateBlocker::SurfaceChanged);
                }
                if live.draft != Draft::Text(action.text.clone()) {
                    blockers.push(SendGateBlocker::DraftMismatch);
                }
            }
        }

        blockers.sort_by_key(|blocker| *blocker as u8);
        blockers.dedup();
        Ok(SendGateReport {
            phase,
            allowed: blockers.is_empty(),
            blockers,
        })
    }

    /// A single owner and &mut borrow serialize GUI calls. Platform workers must not
    /// retain these capabilities after returning or after the process owner exits.
    pub fn dispatch<C: MessageChannel>(
        &mut self,
        action_id: &str,
        now_ms: u64,
        channel: &mut C,
    ) -> Result<ActionState> {
        let (action, state) = self.action(action_id)?;
        if state != ActionState::Prepared {
            return Err(Error::Stale);
        }
        let persistent =
            self.persistent_send_blockers(&action, state, now_ms, &[ActionState::Prepared])?;
        if persistent.contains(&SendGateBlocker::HostPaused) {
            return Err(Error::Blocked("host paused"));
        }
        if persistent.contains(&SendGateBlocker::StaleAction) {
            self.set_action(action_id, ActionState::Stale, "stale_target")?;
            return Ok(ActionState::Stale);
        }
        if !persistent.is_empty() {
            self.set_action(action_id, ActionState::Blocked, "persistent_preflight")?;
            return Ok(ActionState::Blocked);
        }
        let task = task(&self.conn, &action.request_id)?;
        let before = match channel.inspect(&action.target) {
            Ok(live) => live,
            Err(_) => {
                self.set_action(action_id, ActionState::Blocked, "preflight")?;
                return Ok(ActionState::Blocked);
            }
        };
        let preflight = self.evaluate_send_gate(
            action_id,
            now_ms,
            &before,
            SendGatePhase::BeforeFill,
            None,
            &[ActionState::Prepared],
        )?;
        if preflight.blockers.contains(&SendGateBlocker::StaleAction) {
            self.set_action(action_id, ActionState::Stale, "stale_target")?;
            return Ok(ActionState::Stale);
        }
        if !preflight.allowed {
            self.set_action(action_id, ActionState::Blocked, "preflight")?;
            return Ok(ActionState::Blocked);
        }
        {
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            transition(&tx, action_id, ActionState::Executing, "before_side_effect")?;
            tx.execute(
                "UPDATE sessions SET attempts=attempts+1 WHERE key=?1",
                [&task.conversation],
            )?;
            tx.commit()?;
        }
        // Every failure after this point is conservative. No retry, fallback channel,
        // or overwrite of an uncertain draft is hidden in this method.
        if channel.fill(&action, &before).is_err() {
            self.set_action(action_id, ActionState::Unknown, "fill_uncertain")?;
            return Ok(ActionState::Unknown);
        }
        let after = match channel.inspect(&action.target) {
            Ok(live) => live,
            Err(_) => {
                self.set_action(action_id, ActionState::Unknown, "readback_uncertain")?;
                return Ok(ActionState::Unknown);
            }
        };
        let send_gate = self.evaluate_send_gate(
            action_id,
            now_ms,
            &after,
            SendGatePhase::BeforeSend,
            Some(&before),
            &[ActionState::Executing],
        )?;
        if !send_gate.allowed {
            self.set_action(action_id, ActionState::Unknown, "send_gate_uncertain")?;
            return Ok(ActionState::Unknown);
        }
        let evidence = channel
            .send(&action, &after)
            .unwrap_or(SendEvidence::Unknown);
        self.record_evidence(&action, evidence, false)
    }

    /// Explicit recovery for a draft that was written but whose fill readback was uncertain.
    /// This never fills again, never bypasses target/draft checks and is bounded to the same
    /// persisted action. Callers must prove that no external send side effect occurred.
    pub fn recover_filled_send<C: MessageChannel>(
        &mut self,
        action_id: &str,
        now_ms: u64,
        channel: &mut C,
    ) -> Result<ActionState> {
        self.recover_filled_send_for_reason(
            action_id,
            now_ms,
            channel,
            "fill_uncertain",
            "filled_recovery_before_send",
        )
    }

    /// Explicit recovery after a channel returned a trusted pre-send result proving that no
    /// click/key action occurred. The caller must validate the persisted receipt and the
    /// channel attempt report before using this entry point.
    pub fn recover_unattempted_send<C: MessageChannel>(
        &mut self,
        action_id: &str,
        now_ms: u64,
        channel: &mut C,
    ) -> Result<ActionState> {
        self.recover_filled_send_for_reason(
            action_id,
            now_ms,
            channel,
            "effect_unconfirmed",
            "unattempted_recovery_before_send",
        )
    }

    fn recover_filled_send_for_reason<C: MessageChannel>(
        &mut self,
        action_id: &str,
        now_ms: u64,
        channel: &mut C,
        expected_reason: &str,
        transition_reason: &str,
    ) -> Result<ActionState> {
        const MAX_RECOVERY_AGE_MS: u64 = 1_800_000;
        let (action, state) = self.action(action_id)?;
        let reason: Option<String> = self.conn.query_row(
            "SELECT reason FROM outbox WHERE id=?1",
            [action_id],
            |row| row.get(0),
        )?;
        if state != ActionState::Unknown || reason.as_deref() != Some(expected_reason) {
            return Err(Error::Stale);
        }
        let task = task(&self.conn, &action.request_id)?;
        let current = session(&self.conn, &task.conversation)?;
        let expired =
            now_ms < action.created_ms || now_ms - action.created_ms > MAX_RECOVERY_AGE_MS;
        let invalid_target = !current.binding.enabled
            || action.target != current.binding.key
            || action.identity_epoch != current.binding.identity_epoch
            || action.session_revision != current.revision
            || action.profile_version != current.binding.profile_version
            || (current.binding.mode != Mode::AutoReply && !action.approved_by_user);
        let unresolved: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM outbox WHERE conversation=?1 AND id<>?2
             AND state IN ('EXECUTING','SUBMITTED','UNKNOWN'))",
            params![task.conversation, action.action_id],
            |row| row.get(0),
        )?;
        if self.host_is_paused()?
            || expired
            || invalid_target
            || current.attempts == 0
            || current.attempts > current.binding.max_auto_sends
            || unresolved
        {
            return Err(Error::Blocked("filled recovery preflight"));
        }
        let live = channel.inspect(&action.target)?;
        if !live.accepts(&action) || live.draft != Draft::Text(action.text.clone()) {
            return Err(Error::Blocked("filled recovery draft mismatch"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transition(&tx, action_id, ActionState::Executing, transition_reason)?;
        tx.commit()?;
        let evidence = channel
            .send(&action, &live)
            .unwrap_or(SendEvidence::Unknown);
        self.record_evidence(&action, evidence, false)
    }

    pub fn reconcile<C: MessageChannel>(
        &mut self,
        action_id: &str,
        channel: &mut C,
    ) -> Result<ActionState> {
        let (action, state) = self.action(action_id)?;
        if !matches!(state, ActionState::Unknown | ActionState::Submitted) {
            return Err(Error::Stale);
        }
        let evidence = channel.reconcile(&action).unwrap_or(SendEvidence::Unknown);
        self.record_evidence(&action, evidence, true)
    }

    fn record_evidence(
        &mut self,
        action: &OutboundAction,
        evidence: SendEvidence,
        reconciliation: bool,
    ) -> Result<ActionState> {
        let (state, reason) = match evidence {
            SendEvidence::ObservedOutgoing { action_id } if action_id == action.action_id => (
                ActionState::VerifiedOutgoing,
                "matched_outgoing_not_delivery",
            ),
            SendEvidence::Submitted if !reconciliation => {
                (ActionState::Submitted, "channel_submitted_not_delivery")
            }
            _ => (ActionState::Unknown, "effect_unconfirmed"),
        };
        self.set_action(&action.action_id, state, reason)?;
        Ok(state)
    }

    fn set_action(
        &mut self,
        action_id: &str,
        state: ActionState,
        reason: &'static str,
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transition(&tx, action_id, state, reason)?;
        tx.commit()?;
        Ok(())
    }

    pub fn task_state(&self, request_id: &str) -> Result<String> {
        Ok(task(&self.conn, request_id)?.state)
    }

    pub fn transition_history(&self, action_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT state FROM transitions WHERE action_id=?1 ORDER BY seq")?;
        Ok(stmt
            .query_map([action_id], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?)
    }

    pub fn summary(&self) -> Result<Summary> {
        let messages = unsigned(self.conn.query_row(
            "SELECT COUNT(*) FROM messages",
            [],
            |r| r.get(0),
        )?)?;
        let tasks = unsigned(
            self.conn
                .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))?,
        )?;
        let mut stmt = self
            .conn
            .prepare("SELECT id,state FROM outbox ORDER BY rowid")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let actions = rows
            .into_iter()
            .map(|(id, state)| Ok((id, ActionState::parse(&state)?)))
            .collect::<Result<_>>()?;
        Ok(Summary {
            messages,
            tasks,
            actions,
        })
    }
}

fn private_file(path: &Path) -> Result<File> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err(Error::UnsafeState);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o077 != 0 {
                    return Err(Error::UnsafeState);
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn id() -> String {
    Uuid::new_v4().to_string()
}
fn signed(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::Invalid("integer range"))
}
fn unsigned(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::Schema)
}

fn session(conn: &Connection, key: &str) -> Result<Session> {
    let (binding, revision, public_ref, attempts): (String, i64, String, u32) = conn
        .query_row(
            "SELECT binding,revision,public_ref,attempts FROM sessions WHERE key=?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    Ok(Session {
        binding: serde_json::from_str(&binding)?,
        revision: unsigned(revision)?,
        public_ref,
        attempts,
    })
}

fn task(conn: &Connection, request_id: &str) -> Result<Task> {
    let (conversation, request, response, created_ms, state): (
        String,
        String,
        Option<String>,
        i64,
        String,
    ) = conn
        .query_row(
            "SELECT conversation,request,response,created_ms,state FROM tasks WHERE id=?1",
            [request_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    Ok(Task {
        conversation,
        request: serde_json::from_str(&request)?,
        response,
        created_ms: unsigned(created_ms)?,
        state,
    })
}

fn is_current(task: &Task, current: &Session, now_ms: u64) -> bool {
    current.binding.enabled
        && task.request.session_revision == current.revision
        && task.request.provider_profile_version == current.binding.profile_version
        && now_ms >= task.created_ms
        && now_ms - task.created_ms <= current.binding.reply_ttl_ms
}

fn invalidate(conn: &Connection, key: &str, requeue: bool) -> Result<()> {
    conn.execute("INSERT INTO transitions(action_id,state,reason)
        SELECT id,'STALE','session_revision' FROM outbox WHERE conversation=?1 AND state='PREPARED'", [key])?;
    conn.execute("UPDATE outbox SET state='STALE',reason='session_revision' WHERE conversation=?1 AND state='PREPARED'", [key])?;
    conn.execute("UPDATE tasks SET state='STALE' WHERE conversation=?1 AND state IN ('GENERATING','READY','PREPARED')", [key])?;
    conn.execute("UPDATE messages SET disposition=?2,task_id=NULL WHERE conversation=?1 AND disposition='ASSIGNED'
        AND task_id IN (SELECT id FROM tasks WHERE state='STALE')", params![key,if requeue {"QUEUED"} else {"IGNORED"}])?;
    Ok(())
}

fn quarantine(conn: &Connection, key: &str, message_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE messages SET disposition='AMBIGUOUS' WHERE id=?1",
        [message_id],
    )?;
    invalidate(conn, key, true)?;
    conn.execute(
        "UPDATE sessions SET revision=revision+1 WHERE key=?1",
        [key],
    )?;
    Ok(())
}

fn read_events(
    conn: &Connection,
    key: &str,
    queued_only: bool,
    limit: usize,
    through_event: Option<&str>,
) -> Result<Vec<InputEvent>> {
    let epoch = signed(session(conn, key)?.binding.identity_epoch)?;
    let boundary: (i64, i64) = if let Some(event_id) = through_event {
        conn.query_row(
            "SELECT observed_ms,rowid FROM messages WHERE conversation=?1 AND id=?2",
            params![key, event_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
    } else {
        (i64::MAX, i64::MAX)
    };
    let sql = if queued_only {
        "SELECT id,payload,disposition FROM messages WHERE conversation=?1 AND disposition='QUEUED'
         AND identity_epoch=?5 AND (observed_ms < ?3 OR (observed_ms = ?3 AND rowid <= ?4))
         ORDER BY observed_ms,rowid LIMIT ?2"
    } else {
        "SELECT id,payload,disposition FROM (SELECT id,payload,disposition,observed_ms,rowid AS seq
         FROM messages WHERE conversation=?1 AND identity_epoch=?5 AND (observed_ms < ?3 OR (observed_ms = ?3 AND rowid <= ?4))
         ORDER BY observed_ms DESC,rowid DESC LIMIT ?2) ORDER BY observed_ms,seq"
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(
            params![key, limit as i64, boundary.0, boundary.1, epoch],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(event_id, payload, disposition)| {
            let mut message: Message = serde_json::from_str(&payload)?;
            if disposition == "AMBIGUOUS" {
                message.complete = false;
            }
            Ok(InputEvent { event_id, message })
        })
        .collect()
}

fn finish_task(conn: &Connection, request_id: &str, state: &str) -> Result<()> {
    conn.execute(
        "UPDATE tasks SET state=?2 WHERE id=?1",
        params![request_id, state],
    )?;
    conn.execute(
        "UPDATE messages SET disposition='HANDLED' WHERE task_id=?1 AND disposition='ASSIGNED'",
        [request_id],
    )?;
    Ok(())
}

fn transition(conn: &Connection, action_id: &str, state: ActionState, reason: &str) -> Result<()> {
    let task_id: String =
        conn.query_row("SELECT task_id FROM outbox WHERE id=?1", [action_id], |r| {
            r.get(0)
        })?;
    conn.execute(
        "UPDATE outbox SET state=?2,reason=?3 WHERE id=?1",
        params![action_id, state.as_str(), reason],
    )?;
    conn.execute(
        "INSERT INTO transitions(action_id,state,reason) VALUES (?1,?2,?3)",
        params![action_id, state.as_str(), reason],
    )?;
    if matches!(
        state,
        ActionState::VerifiedOutgoing | ActionState::Blocked | ActionState::Stale
    ) {
        finish_task(
            conn,
            &task_id,
            if state == ActionState::VerifiedOutgoing {
                "DONE"
            } else {
                state.as_str()
            },
        )?;
    } else {
        conn.execute(
            "UPDATE tasks SET state=?2 WHERE id=?1",
            params![task_id, state.as_str()],
        )?;
    }
    Ok(())
}

fn valid_reply(text: &str, max_chars: usize) -> bool {
    !text.trim().is_empty()
        && text.chars().count() <= max_chars
        && !text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}
