mod archive;
mod chunking;
mod error;
mod facts;
mod memory_repository;
mod noting;
mod schema;
mod sqlite_vec;
mod time;

pub(crate) use archive::{ArchiveOutcome, ConversationArchive, MemoryEligibility};
pub(crate) use error::StorageError;
#[cfg_attr(
    not(test),
    expect(unused_imports, reason = "fast-memory 04 to 06 use the rest")
)]
pub(crate) use facts::{
    AddedFact, FACT_LINE_OVERHEAD_BYTES, Fact, FactError, FactId, FactOrigin, FactsRepository,
    MAX_FACT_BYTES, MAX_FACTS_BYTES,
};
pub(crate) use memory_repository::*;
pub(crate) use noting::{Examination, TurnId};
