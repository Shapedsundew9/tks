//! In-memory draft session store and background sweep (D-EA-10, D-EA-11, §6.2).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use chrono::Utc;
use deadpool_postgres::Pool;
use tokio::sync::{RwLock, broadcast};
use uuid::Uuid;

use super::config::ElicitationConfig;
use super::session::{
    ElicitationParticipant, ElicitationSession, ElicitationTurn, ImportedGapQuestion, InputMode,
    SessionState, TurnRole, UncertainFlag, normalize_turn_text, validate_participant_name,
    validate_session_slug,
};
use super::transcript::{CloseValidationError, validate_transcript_for_close};
use crate::ingest::elicitation::{ExtractionSessionState, detect_gaps};
use crate::ingest::transcript::ParsedTranscript;

/// Maximum live elicitation draft sessions in memory concurrently.
pub const MAX_LIVE_SESSIONS: usize = 32;

/// Maximum number of turns allowed in a single elicitation session.
pub const MAX_TURNS_PER_SESSION: usize = 1000;

/// Real-time Server-Sent Events emitted by an elicitation session.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", content = "data")]
pub enum ElicitationEvent {
    #[serde(rename = "turn_committed")]
    TurnCommitted(ElicitationTurn),
    #[serde(rename = "turn_inserted")]
    TurnInserted {
        inserted_turn: ElicitationTurn,
        turns: Vec<ElicitationTurn>,
        source: String,
    },
    #[serde(rename = "turn_updated")]
    TurnUpdated(ElicitationTurn),
    #[serde(rename = "turn_deleted")]
    TurnDeleted {
        deleted_number: u32,
        turns: Vec<ElicitationTurn>,
        source: String,
    },
    #[serde(rename = "facilitator_proposal")]
    FacilitatorProposal {
        proposal_id: Uuid,
        mode: String,
        text: String,
        /// Structured ideas for `inspiration` mode (empty for `socratic`).
        #[serde(skip_serializing_if = "Vec::is_empty")]
        ideas: Vec<super::facilitator::InspirationIdea>,
        flags: Vec<super::neutrality::NeutralityFlag>,
        generation: super::facilitator::GenerationMetadata,
    },
    #[serde(rename = "facilitator_error")]
    FacilitatorError { error: String },
    #[serde(rename = "session_closed")]
    SessionClosed { elicitation_id: Uuid },
}

/// Errors returned by the in-memory draft session store.
#[derive(Debug)]
pub enum ElicitationError {
    NotFound(String),
    TurnNotFound(u32),
    SessionClosed,
    Validation(String),
    InvalidPriorTranscript(String),
    CapacityExceeded(String),
    CloseValidation(Vec<CloseValidationError>),
    Database(String),
    Facilitator(String),
}

impl fmt::Display for ElicitationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(msg) => write!(f, "session '{msg}' not found or expired"),
            Self::TurnNotFound(num) => write!(f, "turn '{num}' not found"),
            Self::SessionClosed => write!(f, "cannot modify closed session"),
            Self::Validation(msg) => write!(f, "validation error: {msg}"),
            Self::InvalidPriorTranscript(msg) => write!(f, "prior transcript parse error: {msg}"),
            Self::CapacityExceeded(msg) => write!(f, "capacity exceeded: {msg}"),
            Self::CloseValidation(_) => write!(f, "close validation failed"),
            Self::Database(msg) => write!(f, "database error: {msg}"),
            Self::Facilitator(msg) => write!(f, "facilitator error: {msg}"),
        }
    }
}

impl std::error::Error for ElicitationError {}

/// Request payload to initialize an elicitation session.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateSessionRequest {
    pub title: String,
    #[serde(default)]
    pub session_id: Option<String>,
    pub stakeholder_name: String,
    pub facilitator_name: String,
    #[serde(default)]
    pub prior_transcript: Option<String>,
    #[serde(default)]
    pub prior_extraction_session_id: Option<Uuid>,
}

/// Request payload to commit a conversational turn.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct AddTurnRequest {
    pub role: TurnRole,
    pub text: String,
    pub input_mode: InputMode,
    #[serde(default)]
    pub insert_after_turn: Option<u32>,
}

/// Request payload to patch an existing conversational turn.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct PatchTurnRequest {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub input_mode: Option<InputMode>,
    #[serde(default)]
    pub uncertain_flags: Option<Vec<UncertainFlag>>,
}

/// Shared in-memory store for draft elicitation sessions.
#[derive(Clone)]
pub struct ElicitationStore {
    sessions: Arc<RwLock<HashMap<Uuid, ElicitationSession>>>,
    event_txs: Arc<RwLock<HashMap<Uuid, broadcast::Sender<ElicitationEvent>>>>,
    pub config: ElicitationConfig,
    pub facilitator: Option<Arc<dyn super::FacilitatorProvider>>,
}

impl ElicitationStore {
    /// Creates a new `ElicitationStore` with the given configuration.
    #[must_use]
    pub fn new(config: ElicitationConfig) -> Self {
        let facilitator = if config.facilitator_provider == super::FacilitatorProviderKind::Ollama {
            Some(Arc::new(super::OllamaFacilitatorProvider::new(
                config.ollama_url.clone(),
                config.facilitator_model.clone(),
            )) as Arc<dyn super::FacilitatorProvider>)
        } else {
            None
        };
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            event_txs: Arc::new(RwLock::new(HashMap::new())),
            config,
            facilitator,
        }
    }

    /// Initializes hosted routing once, before exposing the gateway.
    pub async fn initialize_facilitator(&mut self) -> tierfall::Result<()> {
        if self.config.facilitator_provider == super::FacilitatorProviderKind::Tierfall {
            let client = tierfall::Tierfall::from_env().await?;
            self.set_tierfall(client)?;
        }
        Ok(())
    }

    /// Injects a router, including isolated stub routers in tests.
    pub fn set_tierfall(&mut self, client: tierfall::Tierfall) -> tierfall::Result<()> {
        client.resolve(&self.config.facilitator_route)?;
        self.config.facilitator_provider = super::FacilitatorProviderKind::Tierfall;
        self.facilitator = Some(Arc::new(super::TierfallFacilitatorProvider {
            client,
            route: self.config.facilitator_route.clone(),
        }));
        Ok(())
    }

    /// Creates a new elicitation draft session.
    ///
    /// # Errors
    /// Returns [`ElicitationError`] on capacity breach, validation failure, invalid prior transcript,
    /// or gap extraction query errors.
    pub async fn create_session(
        &self,
        req: CreateSessionRequest,
        pool: Option<&Pool>,
    ) -> Result<ElicitationSession, ElicitationError> {
        self.sweep_expired().await;

        let mut sessions_guard = self.sessions.write().await;
        if sessions_guard.len() >= MAX_LIVE_SESSIONS {
            return Err(ElicitationError::CapacityExceeded(format!(
                "maximum live sessions ({MAX_LIVE_SESSIONS}) reached"
            )));
        }

        let mut title = req.title.trim().to_string();
        let mut session_id = req.session_id.map(|s| s.trim().to_string());
        let mut stakeholder_name = req.stakeholder_name.trim().to_string();
        let mut facilitator_name = req.facilitator_name.trim().to_string();
        let mut cycle = 1u32;

        // Cycle continuation (D-134)
        if let Some(ref prior_md) = req.prior_transcript {
            let parsed_prior = ParsedTranscript::parse(prior_md)
                .map_err(|e| ElicitationError::InvalidPriorTranscript(e.to_string()))?;

            if session_id.is_none() || session_id.as_deref() == Some("") {
                session_id = Some(parsed_prior.frontmatter.session_id.clone());
            }
            cycle = parsed_prior.frontmatter.cycle + 1;

            if title.is_empty() {
                title = parsed_prior.frontmatter.title;
            }

            if stakeholder_name.is_empty()
                && let Some(p) = parsed_prior
                    .frontmatter
                    .participants
                    .iter()
                    .find(|p| p.role == "stakeholder")
            {
                stakeholder_name = p.name.clone();
            }

            if facilitator_name.is_empty()
                && let Some(p) = parsed_prior
                    .frontmatter
                    .participants
                    .iter()
                    .find(|p| p.role == "facilitator")
            {
                facilitator_name = p.name.clone();
            }
        }

        let session_id_str = session_id.ok_or_else(|| {
            ElicitationError::Validation("session_id is required for new sessions".to_string())
        })?;

        if !validate_session_slug(&session_id_str) {
            return Err(ElicitationError::Validation(format!(
                "session_id '{session_id_str}' does not match slug pattern"
            )));
        }

        if title.is_empty() {
            return Err(ElicitationError::Validation(
                "title cannot be empty".to_string(),
            ));
        }

        validate_participant_name(&stakeholder_name)
            .map_err(|e| ElicitationError::Validation(format!("stakeholder name error: {e}")))?;
        validate_participant_name(&facilitator_name)
            .map_err(|e| ElicitationError::Validation(format!("facilitator name error: {e}")))?;

        if stakeholder_name.eq_ignore_ascii_case(&facilitator_name) {
            return Err(ElicitationError::Validation(
                "stakeholder and facilitator names must be distinct".to_string(),
            ));
        }

        // Gap import
        let mut imported_gap_questions = Vec::new();
        if let Some(ext_id) = req.prior_extraction_session_id {
            let pool = pool.ok_or_else(|| {
                ElicitationError::Database("database pool unavailable for gap import".to_string())
            })?;
            let client = pool
                .get()
                .await
                .map_err(|e| ElicitationError::Database(format!("pool error: {e}")))?;

            let row = client
                .query_opt(
                    "SELECT attributes FROM ingestion_jobs WHERE job_id = $1;",
                    &[&ext_id],
                )
                .await
                .map_err(|e| ElicitationError::Database(format!("query error: {e}")))?
                .ok_or_else(|| {
                    ElicitationError::NotFound(format!("extraction session '{ext_id}' not found"))
                })?;

            let attr_val: serde_json::Value = row.get("attributes");
            let ext_state: ExtractionSessionState = serde_json::from_value(attr_val)
                .map_err(|e| ElicitationError::Database(format!("deserialization error: {e}")))?;

            let gaps = if !ext_state.gaps.is_empty() {
                ext_state.gaps.clone()
            } else {
                detect_gaps(&ext_state)
            };

            for gap in gaps {
                imported_gap_questions.push(ImportedGapQuestion {
                    gap_id: gap.id,
                    detector: gap.detector,
                    suggested_question: gap.suggested_question,
                });
            }
        }

        let now = Utc::now();
        let elicitation_id = Uuid::new_v4();
        let expires_at = now
            + chrono::Duration::from_std(self.config.idle_expiry)
                .unwrap_or(chrono::Duration::days(1));

        let session = ElicitationSession {
            elicitation_id,
            state: SessionState::Open,
            title,
            session_id: session_id_str.clone(),
            cycle,
            date: now.format("%Y-%m-%d").to_string(),
            language: "en".to_string(),
            participants: vec![
                ElicitationParticipant {
                    name: stakeholder_name,
                    role: "stakeholder".to_string(),
                },
                ElicitationParticipant {
                    name: facilitator_name,
                    role: "facilitator".to_string(),
                },
            ],
            turns: Vec::new(),
            imported_gap_questions,
            explored_items: Vec::new(),
            created_at: now,
            last_activity_at: now,
            expires_at,
            source: "manual".to_string(),
        };

        sessions_guard.insert(elicitation_id, session.clone());

        let (tx, _) = broadcast::channel(64);
        let mut txs_guard = self.event_txs.write().await;
        txs_guard.insert(elicitation_id, tx);

        tracing::info!(
            elicitation_id = %elicitation_id,
            session_id = %session_id_str,
            cycle = cycle,
            "Created elicitation draft session"
        );

        Ok(session)
    }

    /// Fetches an elicitation draft session by its unique ID.
    ///
    /// # Errors
    /// Returns [`ElicitationError::NotFound`] if the session does not exist or has expired.
    pub async fn get_session(
        &self,
        elicitation_id: Uuid,
    ) -> Result<ElicitationSession, ElicitationError> {
        let mut sessions_guard = self.sessions.write().await;
        let session = sessions_guard
            .get_mut(&elicitation_id)
            .ok_or_else(|| ElicitationError::NotFound(elicitation_id.to_string()))?;

        let now = Utc::now();
        let idle_dur = chrono::Duration::from_std(self.config.idle_expiry)
            .unwrap_or(chrono::Duration::days(1));

        if now > session.last_activity_at + idle_dur {
            sessions_guard.remove(&elicitation_id);
            drop(sessions_guard);
            let mut txs = self.event_txs.write().await;
            txs.remove(&elicitation_id);
            return Err(ElicitationError::NotFound(elicitation_id.to_string()));
        }

        session.expires_at = session.last_activity_at + idle_dur;
        session.source = session.compute_source().to_string();

        Ok(session.clone())
    }

    /// Appends a new conversational turn to the session.
    ///
    /// # Errors
    /// Returns [`ElicitationError`] if the session is closed, turn bounds are exceeded, or
    /// text normalisation fails.
    pub async fn add_turn(
        &self,
        elicitation_id: Uuid,
        req: AddTurnRequest,
    ) -> Result<ElicitationTurn, ElicitationError> {
        let normalized_text = normalize_turn_text(&req.text)
            .map_err(|e| ElicitationError::Validation(e.to_string()))?;

        let mut sessions_guard = self.sessions.write().await;
        let session = sessions_guard
            .get_mut(&elicitation_id)
            .ok_or_else(|| ElicitationError::NotFound(elicitation_id.to_string()))?;

        if session.state == SessionState::Closed {
            return Err(ElicitationError::SessionClosed);
        }

        if session.turns.len() >= MAX_TURNS_PER_SESSION {
            return Err(ElicitationError::CapacityExceeded(format!(
                "maximum turns ({MAX_TURNS_PER_SESSION}) reached for session"
            )));
        }

        let insert_idx = if let Some(after_num) = req.insert_after_turn {
            if let Some(pos) = session.turns.iter().position(|t| t.number == after_num) {
                pos + 1
            } else {
                session.turns.len()
            }
        } else {
            session.turns.len()
        };

        let now = Utc::now();
        let number = (insert_idx + 1) as u32;
        let mut turn = ElicitationTurn {
            number,
            role: req.role,
            text: normalized_text,
            input_mode: req.input_mode,
            uncertain_flags: Vec::new(),
            committed_at: now,
            edited_at: None,
        };

        let is_middle_insert = insert_idx < session.turns.len();
        if is_middle_insert {
            session.turns.insert(insert_idx, turn.clone());
            for (idx, t) in session.turns.iter_mut().enumerate() {
                t.number = (idx + 1) as u32;
            }
            turn.number = (insert_idx + 1) as u32;
        } else {
            session.turns.push(turn.clone());
        }

        session.last_activity_at = now;
        session.expires_at = now
            + chrono::Duration::from_std(self.config.idle_expiry)
                .unwrap_or(chrono::Duration::days(1));
        let computed_source = session.compute_source().to_string();
        session.source.clone_from(&computed_source);
        let all_turns = session.turns.clone();

        drop(sessions_guard);

        let txs = self.event_txs.read().await;
        if let Some(tx) = txs.get(&elicitation_id) {
            if is_middle_insert {
                let _ = tx.send(ElicitationEvent::TurnInserted {
                    inserted_turn: turn.clone(),
                    turns: all_turns,
                    source: computed_source,
                });
            } else {
                let _ = tx.send(ElicitationEvent::TurnCommitted(turn.clone()));
            }
        }

        tracing::info!(
            elicitation_id = %elicitation_id,
            turn_number = turn.number,
            role = %turn.role,
            input_mode = %turn.input_mode,
            is_middle_insert = is_middle_insert,
            "Turn committed"
        );

        Ok(turn)
    }

    /// Updates an existing committed conversational turn (transcription correction or flags).
    ///
    /// # Errors
    /// Returns [`ElicitationError`] if the session is closed or turn is missing.
    pub async fn patch_turn(
        &self,
        elicitation_id: Uuid,
        number: u32,
        patch: PatchTurnRequest,
    ) -> Result<ElicitationTurn, ElicitationError> {
        let mut sessions_guard = self.sessions.write().await;
        let session = sessions_guard
            .get_mut(&elicitation_id)
            .ok_or_else(|| ElicitationError::NotFound(elicitation_id.to_string()))?;

        if session.state == SessionState::Closed {
            return Err(ElicitationError::SessionClosed);
        }

        let now = Utc::now();
        let updated_turn = {
            let turn = session
                .turns
                .iter_mut()
                .find(|t| t.number == number)
                .ok_or(ElicitationError::TurnNotFound(number))?;

            if let Some(new_text) = patch.text {
                let normalized = normalize_turn_text(&new_text)
                    .map_err(|e| ElicitationError::Validation(e.to_string()))?;
                turn.text = normalized;
                turn.edited_at = Some(now);
            }

            // A dictated mode is never downgraded to typed (D-EA-15, §6.2)
            if let Some(new_mode) = patch.input_mode
                && (turn.input_mode != InputMode::Dictated || new_mode != InputMode::Typed)
            {
                turn.input_mode = new_mode;
            }

            if let Some(flags) = patch.uncertain_flags {
                turn.uncertain_flags = flags;
            }

            turn.clone()
        };

        session.last_activity_at = now;
        session.expires_at = now
            + chrono::Duration::from_std(self.config.idle_expiry)
                .unwrap_or(chrono::Duration::days(1));
        session.source = session.compute_source().to_string();

        drop(sessions_guard);

        let txs = self.event_txs.read().await;
        if let Some(tx) = txs.get(&elicitation_id) {
            let _ = tx.send(ElicitationEvent::TurnUpdated(updated_turn.clone()));
        }

        tracing::info!(
            elicitation_id = %elicitation_id,
            turn_number = number,
            "Turn updated"
        );

        Ok(updated_turn)
    }

    /// Deletes a committed conversational turn and re-numbers remaining turns contiguously (`1..=N`).
    ///
    /// # Errors
    /// Returns [`ElicitationError`] if the session is closed or the turn is missing.
    pub async fn delete_turn(
        &self,
        elicitation_id: Uuid,
        number: u32,
    ) -> Result<Vec<ElicitationTurn>, ElicitationError> {
        let mut sessions_guard = self.sessions.write().await;
        let session = sessions_guard
            .get_mut(&elicitation_id)
            .ok_or_else(|| ElicitationError::NotFound(elicitation_id.to_string()))?;

        if session.state == SessionState::Closed {
            return Err(ElicitationError::SessionClosed);
        }

        let pos = session
            .turns
            .iter()
            .position(|t| t.number == number)
            .ok_or(ElicitationError::TurnNotFound(number))?;

        session.turns.remove(pos);

        for (idx, turn) in session.turns.iter_mut().enumerate() {
            turn.number = (idx + 1) as u32;
        }

        let now = Utc::now();
        session.last_activity_at = now;
        session.expires_at = now
            + chrono::Duration::from_std(self.config.idle_expiry)
                .unwrap_or(chrono::Duration::days(1));
        let computed_source = session.compute_source().to_string();
        session.source.clone_from(&computed_source);
        let remaining_turns = session.turns.clone();

        drop(sessions_guard);

        let txs = self.event_txs.read().await;
        if let Some(tx) = txs.get(&elicitation_id) {
            let _ = tx.send(ElicitationEvent::TurnDeleted {
                deleted_number: number,
                turns: remaining_turns.clone(),
                source: computed_source,
            });
        }

        tracing::info!(
            elicitation_id = %elicitation_id,
            deleted_turn_number = number,
            remaining_count = remaining_turns.len(),
            "Turn deleted and remaining turns re-indexed"
        );

        Ok(remaining_turns)
    }

    /// Records explored or dismissed idea titles / Socratic questions in the session
    /// so subsequent LLM calls avoid repeating them.
    pub async fn record_explored_items(&self, elicitation_id: Uuid, items: Vec<String>) {
        if items.is_empty() {
            return;
        }
        let mut sessions_guard = self.sessions.write().await;
        if let Some(session) = sessions_guard.get_mut(&elicitation_id) {
            for item in items {
                let cleaned = item.trim().to_string();
                if !cleaned.is_empty()
                    && !session
                        .explored_items
                        .iter()
                        .any(|existing| existing.eq_ignore_ascii_case(&cleaned))
                {
                    session.explored_items.push(cleaned);
                }
            }
            if session.explored_items.len() > 128 {
                let excess = session.explored_items.len() - 128;
                session.explored_items.drain(0..excess);
            }
        }
    }

    /// Validates and closes a session.
    ///
    /// # Errors
    /// Returns [`ElicitationError::CloseValidation`] if export validation fails.
    pub async fn close_session(&self, elicitation_id: Uuid) -> Result<(), ElicitationError> {
        let mut sessions_guard = self.sessions.write().await;
        let session = sessions_guard
            .get_mut(&elicitation_id)
            .ok_or_else(|| ElicitationError::NotFound(elicitation_id.to_string()))?;

        session.source = session.compute_source().to_string();

        validate_transcript_for_close(session).map_err(ElicitationError::CloseValidation)?;

        session.state = SessionState::Closed;
        session.last_activity_at = Utc::now();

        drop(sessions_guard);

        let txs = self.event_txs.read().await;
        if let Some(tx) = txs.get(&elicitation_id) {
            let _ = tx.send(ElicitationEvent::SessionClosed { elicitation_id });
        }

        tracing::info!(elicitation_id = %elicitation_id, "Session closed");

        Ok(())
    }

    /// Reopens a closed session for further edits or turns.
    ///
    /// # Errors
    /// Returns [`ElicitationError::NotFound`] if the session is not found.
    pub async fn reopen_session(
        &self,
        elicitation_id: Uuid,
    ) -> Result<ElicitationSession, ElicitationError> {
        let mut sessions_guard = self.sessions.write().await;
        let session = sessions_guard
            .get_mut(&elicitation_id)
            .ok_or_else(|| ElicitationError::NotFound(elicitation_id.to_string()))?;

        session.state = SessionState::Open;
        session.last_activity_at = Utc::now();
        session.expires_at = session.last_activity_at
            + chrono::Duration::from_std(self.config.idle_expiry)
                .unwrap_or(chrono::Duration::days(1));

        tracing::info!(elicitation_id = %elicitation_id, "Session reopened");

        Ok(session.clone())
    }

    /// Immediately discards an elicitation draft session.
    ///
    /// # Errors
    /// Returns [`ElicitationError::NotFound`] if the session does not exist.
    pub async fn delete_session(&self, elicitation_id: Uuid) -> Result<(), ElicitationError> {
        let mut sessions_guard = self.sessions.write().await;
        let removed = sessions_guard.remove(&elicitation_id);
        drop(sessions_guard);

        let mut txs = self.event_txs.write().await;
        txs.remove(&elicitation_id);

        if removed.is_some() {
            tracing::info!(elicitation_id = %elicitation_id, "Session deleted");
            Ok(())
        } else {
            Err(ElicitationError::NotFound(elicitation_id.to_string()))
        }
    }

    /// Subscribes to real-time events for a session.
    ///
    /// # Errors
    /// Returns [`ElicitationError::NotFound`] if the session is not found.
    pub async fn subscribe_events(
        &self,
        elicitation_id: Uuid,
    ) -> Result<broadcast::Receiver<ElicitationEvent>, ElicitationError> {
        let txs = self.event_txs.read().await;
        let tx = txs
            .get(&elicitation_id)
            .ok_or_else(|| ElicitationError::NotFound(elicitation_id.to_string()))?;
        Ok(tx.subscribe())
    }

    /// Broadcasts a facilitator proposal or error event.
    pub async fn broadcast_event(&self, elicitation_id: Uuid, event: ElicitationEvent) {
        let txs = self.event_txs.read().await;
        if let Some(tx) = txs.get(&elicitation_id) {
            let _ = tx.send(event);
        }
    }

    /// Sweeps sessions that have been idle longer than the configured expiry duration.
    pub async fn sweep_expired(&self) -> usize {
        let now = Utc::now();
        let idle_dur = chrono::Duration::from_std(self.config.idle_expiry)
            .unwrap_or(chrono::Duration::days(1));

        let mut to_remove = Vec::new();
        {
            let sessions = self.sessions.read().await;
            for (id, session) in sessions.iter() {
                if now > session.last_activity_at + idle_dur {
                    to_remove.push(*id);
                }
            }
        }

        if !to_remove.is_empty() {
            let mut sessions = self.sessions.write().await;
            let mut txs = self.event_txs.write().await;
            for id in &to_remove {
                sessions.remove(id);
                txs.remove(id);
            }
            tracing::debug!(
                expired_count = to_remove.len(),
                "Swept expired elicitation sessions"
            );
        }

        to_remove.len()
    }
}
