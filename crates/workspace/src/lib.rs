//! Durable workspace and snapshot boundaries.
#![forbid(unsafe_code)]

mod error;
mod event;
mod service;
mod state;

pub use error::WorkspaceError;
pub use event::{WorkspaceMutationV1, WorkspaceOperationV1};
pub use service::{WorkspaceCheckpoint, WorkspaceService};
pub use state::{WorkspaceFile, WorkspaceState};
