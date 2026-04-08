//! Storage section abstractions for the `.grafeo` container format.
//!
//! The container file stores data in typed **sections**, each independently
//! addressable and checksummed. This module defines the contract between
//! section serializers (in `grafeo-core`) and section I/O (in `grafeo-storage`).

pub mod section;

pub use section::{Section, SectionDirectoryEntry, SectionFlags, SectionType};
