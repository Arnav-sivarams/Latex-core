#![forbid(unsafe_code)]
//! Best-effort static LaTeX analysis. Successful parsing is not a prerequisite for TeX
//! compilation, and actual TeX engines remain the semantic authority.

mod bibtex;
mod diagnostic;
mod edit;
mod error;
mod project;
mod queries;
mod range;
mod semantic;
mod session;

pub use bibtex::*;
pub use diagnostic::*;
pub use edit::*;
pub use error::*;
pub use project::*;
pub use range::*;
pub use semantic::*;
pub use session::*;
