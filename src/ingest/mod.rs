//! Document ingestion, AST decomposition, and span extraction modules.

pub mod matcher;
pub mod parser;
pub mod span;

pub use matcher::*;
pub use parser::*;
pub use span::*;
