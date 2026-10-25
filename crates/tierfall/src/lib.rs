//! # tierfall
//!
//! One OpenAI-compatible endpoint over many free-tier LLM accounts (Google Gemini, Groq,
//! OpenRouter, any OpenAI-compatible server, and local Ollama). Requests walk a quality-ordered
//! fallback chain of `(account, model)` targets; per-minute and daily limits are detected from
//! provider responses, remembered across restarts, and skipped until they reset.
//!
//! ```no_run
//! # async fn demo() -> tierfall::Result<()> {
//! let tf = tierfall::Tierfall::from_env().await?;
//! let reply = tf
//!     .chat(tierfall::ChatRequest::new("auto").user("Say hello in five words."))
//!     .await?;
//! println!("{} (via {:?})", reply.text().unwrap_or_default(), reply.served_by);
//! # Ok(()) }
//! ```
//!
//! Streaming falls back transparently until the first token arrives:
//!
//! ```no_run
//! use futures_util::StreamExt;
//! # async fn demo(tf: tierfall::Tierfall) -> tierfall::Result<()> {
//! let request = tierfall::ChatRequest::new("fast")
//!     .user("Tell me a short joke.")
//!     .param("temperature", 0.7);
//! let mut stream = tf.chat_stream(request).await?;
//! eprintln!("served by {}", stream.served_by().model);
//! while let Some(chunk) = stream.next().await {
//!     print!("{}", chunk?.text().unwrap_or_default());
//! }
//! # Ok(()) }
//! ```

pub mod classify;
pub mod config;
pub mod discovery;
pub mod error;
pub mod provider;
pub mod router;
pub mod routes;
#[cfg(feature = "server")]
pub mod server;
pub mod sse;
pub mod state;
pub mod types;

pub use config::{Account, Config, Env};
pub use error::{Attempt, Error, Result};
pub use provider::ProviderKind;
pub use router::{ChatStream, RawCompletion, StatusReport, Tierfall};
pub use routes::Target;
pub use types::{ChatChunk, ChatRequest, ChatResponse, Message, ServedBy, Usage};
