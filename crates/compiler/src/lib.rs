#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    reason = "typed public errors are self-describing"
)]
//! Manual, isolated TeX Live and latexmk compiler orchestration.

mod container;
mod error;
mod materialize;
mod model;
mod profile;
mod service;
mod synctex;

pub use container::{CompileContainerRequest, ContainerOutput, ContainerRuntime, DockerCliRuntime};
pub use error::CompilerError;
pub use model::{CompileExecution, CompileLimits, CompileStatus, CompilerArtifact};
pub use profile::profile_for;
pub use service::{CompilerConfig, CompilerService};
pub use synctex::{SyncTexIndex, SyncTexLocation};
