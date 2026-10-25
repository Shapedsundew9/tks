//! Intent Elicitation Track (Track E-A) domain logic (D-124..D-134, C-27, C-28).

pub mod assets;
pub mod cli_provider;
pub mod config;
pub mod facilitator;
pub mod neutrality;
pub mod prompts;
pub mod session;
pub mod store;
pub mod transcript;

pub use assets::{AsrFileEntry, AsrLockManifest};
pub use cli_provider::{
    AgyCliAdapter, CliBackendKind, CliFacilitatorProvider, CliModelInfo, CliRole, CliRoleSession,
    CliToolAdapter, CliToolStatus, CliTurnResult, build_cli_turn_prompt, parse_agy_models_output,
    rank_cli_model,
};
pub use config::{ElicitationConfig, FacilitatorProviderKind};
pub use facilitator::{
    FacilitatorContext, FacilitatorProposal, FacilitatorProvider, GenerationMetadata,
    InspirationIdea, NoneFacilitatorProvider, OllamaFacilitatorProvider,
    TierfallFacilitatorProvider, build_facilitator_directive, build_facilitator_user_prompt,
    extract_proposal_items, parse_inspiration_ideas, probe_ollama_status, render_inspiration_ideas,
    resolve_ollama_url,
};
pub use neutrality::{NeutralityFlag, NeutralityFlagCategory, lint_neutrality};
pub use prompts::{
    FACILITATOR_SYSTEM_PROMPT_V1, FACILITATOR_SYSTEM_PROMPT_V2, INSPIRATION_SYSTEM_PROMPT_V1,
    INSPIRATION_SYSTEM_PROMPT_V2, INSPIRATION_SYSTEM_PROMPT_V3,
};
pub use session::{
    ElicitationParticipant, ElicitationSession, ElicitationTurn, ImportedGapQuestion, InputMode,
    SessionState, TurnRole, UncertainFlag,
};
pub use store::{
    ActiveBackendState, AddTurnRequest, CreateSessionRequest, ElicitationError, ElicitationEvent,
    ElicitationStore, PatchTurnRequest,
};
pub use transcript::{CloseValidationError, render_transcript, validate_transcript_for_close};
