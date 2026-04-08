//! Async spill file management (moved from grafeo-core to keep grafeo-core free of tokio).
//!
//! The sync spill infrastructure (`SpillManager`, `SpillFile`, `ExternalSort`)
//! remains in `grafeo-core::execution::spill`. These async wrappers live here
//! because they require a tokio runtime.

pub mod async_file;
pub mod async_manager;

pub use async_file::{AsyncSpillFile, AsyncSpillFileReader};
pub use async_manager::AsyncSpillManager;
