//! Document ingestion, AST decomposition, and span extraction modules.

pub mod classify;
pub mod fallback;
pub mod matcher;
pub mod parser;
pub mod span;

pub use classify::*;
pub use fallback::*;
pub use matcher::*;
pub use parser::*;
pub use span::*;
