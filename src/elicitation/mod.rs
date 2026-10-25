//! Intent Elicitation Track (Track E-A) domain logic (D-124..D-134, C-27, C-28).

pub mod assets;
pub mod config;
pub mod facilitator;
pub mod neutrality;
pub mod prompts;
pub mod session;
pub mod store;
pub mod transcript;

pub use assets::{AsrFileEntry, AsrLockManifest};
pub use config::{ElicitationConfig, FacilitatorProviderKind};
pub use facilitator::{
    FacilitatorContext, FacilitatorProvider, NoneFacilitatorProvider, OllamaFacilitatorProvider,
    build_facilitator_user_prompt, extract_proposal_items, resolve_ollama_url,
};
pub use neutrality::{NeutralityFlag, NeutralityFlagCategory, lint_neutrality};
pub use prompts::{
    FACILITATOR_SYSTEM_PROMPT_V1, FACILITATOR_SYSTEM_PROMPT_V2, INSPIRATION_SYSTEM_PROMPT_V1,
    INSPIRATION_SYSTEM_PROMPT_V2,
};
pub use session::{
    ElicitationParticipant, ElicitationSession, ElicitationTurn, ImportedGapQuestion, InputMode,
    SessionState, TurnRole, UncertainFlag,
};
pub use store::{
    AddTurnRequest, CreateSessionRequest, ElicitationError, ElicitationEvent, ElicitationStore,
    PatchTurnRequest,
};
pub use transcript::{CloseValidationError, render_transcript, validate_transcript_for_close};
