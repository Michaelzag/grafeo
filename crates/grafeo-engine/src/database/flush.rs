//! Unified flush: one code path for checkpoint, eviction, and explicit CHECKPOINT.
//!
//! The [`FlushManager`] replaces separate checkpoint/snapshot/close paths with
//! a single `flush()` method. Three triggers, one implementation:
//!
//! | Trigger | What gets written | RAM after flush |
//! |---------|-------------------|-----------------|
//! | Periodic checkpoint | All dirty sections | Kept |
//! | Memory pressure | Lowest-priority section | Mmap back, release RAM |
//! | Explicit CHECKPOINT | All sections | Kept |
//!
//! Future phases will add memory pressure integration with BufferManager.

use grafeo_common::storage::{Section, SectionType};
use grafeo_common::utils::error::Result;

#[cfg(feature = "grafeo-file")]
use grafeo_storage::file::GrafeoFileManager;

/// Reason for triggering a flush.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushReason {
    /// Periodic checkpoint (timer-driven) or database close.
    Checkpoint,
    /// User-initiated `CHECKPOINT` command or `wal_checkpoint()` API.
    Explicit,
    /// BufferManager memory pressure (future: evict lowest-priority section).
    #[allow(dead_code)]
    MemoryPressure,
}

/// Context needed by each section during serialization.
pub(super) struct FlushContext {
    pub epoch: u64,
    pub transaction_id: u64,
    pub node_count: u64,
    pub edge_count: u64,
}

/// Executes the unified flush: serialize dirty sections, write to container, truncate WAL.
///
/// This is the single write path for all persistence operations.
///
/// # Errors
///
/// Returns an error if serialization or I/O fails.
#[cfg(feature = "grafeo-file")]
pub(super) fn flush(
    fm: &GrafeoFileManager,
    sections: &[&dyn Section],
    context: &FlushContext,
    reason: FlushReason,
    #[cfg(feature = "wal")] wal: Option<&grafeo_storage::wal::LpgWal>,
) -> Result<()> {
    use grafeo_common::testing::crash::maybe_crash;

    maybe_crash("flush:before_serialize");

    // Collect sections to write based on flush reason
    let targets: Vec<(SectionType, Vec<u8>)> = match reason {
        FlushReason::Checkpoint | FlushReason::Explicit => {
            // Write all sections (dirty or not for Explicit, only dirty for Checkpoint)
            let mut result = Vec::new();
            for section in sections {
                if reason == FlushReason::Explicit || section.is_dirty() {
                    result.push((section.section_type(), section.serialize()?));
                }
            }
            // If nothing is dirty on a periodic checkpoint, still write all sections
            // to ensure the container has a complete state for crash recovery.
            if result.is_empty() {
                for section in sections {
                    result.push((section.section_type(), section.serialize()?));
                }
            }
            result
        }
        FlushReason::MemoryPressure => {
            // Future: pick lowest-priority dirty section only.
            // For now, flush all (same as Checkpoint).
            sections
                .iter()
                .map(|s| Ok((s.section_type(), s.serialize()?)))
                .collect::<Result<Vec<_>>>()?
        }
    };

    maybe_crash("flush:after_serialize");

    // Write sections to container
    let section_refs: Vec<(SectionType, &[u8])> =
        targets.iter().map(|(t, d)| (*t, d.as_slice())).collect();

    fm.write_sections(
        &section_refs,
        context.epoch,
        context.transaction_id,
        context.node_count,
        context.edge_count,
    )?;

    // Mark all written sections as clean
    for section in sections {
        if targets.iter().any(|(t, _)| *t == section.section_type()) {
            section.mark_clean();
        }
    }

    maybe_crash("flush:after_write");

    // Truncate WAL (all data is now in the container)
    #[cfg(feature = "wal")]
    if let Some(wal) = wal {
        wal.sync()?;
    }

    Ok(())
}

/// Builds the flush context from the current database state.
pub(super) fn build_context(
    store: &grafeo_core::graph::lpg::LpgStore,
    transaction_manager: &crate::transaction::TransactionManager,
) -> FlushContext {
    FlushContext {
        epoch: store.current_epoch().0,
        transaction_id: transaction_manager
            .last_assigned_transaction_id()
            .map_or(0, |t| t.0),
        node_count: store.node_count() as u64,
        edge_count: store.edge_count() as u64,
    }
}
