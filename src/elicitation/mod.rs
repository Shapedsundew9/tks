//! Intent Elicitation Track (Track E-A) domain logic (D-124..D-134, C-27, C-28).

pub mod config;
pub mod facilitator;
pub mod session;
pub mod store;
pub mod transcript;

pub use config::{ElicitationConfig, FacilitatorProviderKind};
pub use facilitator::{
    FacilitatorContext, FacilitatorProvider, NoneFacilitatorProvider, OllamaFacilitatorProvider,
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
