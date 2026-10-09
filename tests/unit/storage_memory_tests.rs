use std::{error::Error, path::PathBuf};

use super::{
    ArchiveOutcome, CommitDisposition, ConversationArchive, CorrectionEvidence, CorrectionMarker,
    CorrectionState, EvidenceInput, MemoryAttribution, MemoryEligibility, MemoryListFilter,
    MemoryRecordKind, MemoryRecordStatus, NewMemory, ProjectionOutcome, SourceChunk,
    SourcePartRole, SourceReceiptState, StorageError,
};

fn database_path(label: &str) -> PathBuf {
    let name = format!(
        "jarvis-storage-memory-{label}-{}-{}.sqlite3",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos()
    );
    std::env::temp_dir().join(name)
}

fn remove_database(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

fn evidence_span(text: &str, quote: &str) -> EvidenceInput {
    let start = text.find(quote).expect("fixture quote occurs") as u32;
    EvidenceInput {
        part_index: 0,
        start_byte: start,
        end_byte: start + quote.len() as u32,
        quote: quote.to_owned(),
    }
}

fn correction_evidence(
    text: &str,
    marker: CorrectionMarker,
    subject: &str,
    property: &str,
    old_value: &str,
    new_value: &str,
) -> CorrectionEvidence {
    CorrectionEvidence {
        marker,
        subject: evidence_span(text, subject),
        property: evidence_span(text, property),
        old_value: evidence_span(text, old_value),
        new_value: evidence_span(text, new_value),
    }
}

async fn commit_user_memory(
    archive: &ConversationArchive,
    quote: &str,
    kind: MemoryRecordKind,
    correction: Option<CorrectionEvidence>,
    correction_needs_review: bool,
) -> Result<(super::SourceId, super::MemorySummary), Box<dyn Error>> {
    let receipt = archive
        .append_turn_with_memory(
            quote,
            ArchiveOutcome::Completed("Noted.".to_owned()),
            MemoryEligibility::Eligible,
        )
        .await?;
    let source_id = receipt.source_id.expect("eligible turn has a source");
    let repository = archive.memory_repository();
    let lease = repository
        .claim_next_job()
        .await?
        .expect("source extraction is queued");
    assert_eq!(lease.source_id, source_id);
    repository
        .commit_extraction(
            lease,
            vec![NewMemory {
                text: quote.to_owned(),
                kind,
                attribution: MemoryAttribution::UserStatement,
                evidence: vec![evidence_span(quote, quote)],
                correction,
                correction_needs_review,
            }],
        )
        .await?;
    let memory = repository
        .list_memories(MemoryListFilter {
            query: None,
            status: Some(MemoryRecordStatus::Active),
            limit: 50,
        })
        .await?
        .into_iter()
        .find(|memory| memory.source_id == source_id)
        .expect("extraction creates one memory record");
    Ok((source_id, memory))
}

#[tokio::test]
async fn completed_turn_is_grounded_and_source_forget_invalidates_queued_work()
-> Result<(), Box<dyn Error>> {
    let path = database_path("forget-pending");
    let archive = ConversationArchive::open(&path).await?;
    let receipt = archive
        .append_turn_with_memory(
            "I prefer tea in the morning.",
            ArchiveOutcome::Completed("I will keep that in mind.".to_owned()),
            MemoryEligibility::Eligible,
        )
        .await?;
    let source_id = receipt.source_id.expect("eligible success has a source");
    assert_eq!(receipt.memory, super::MemoryEnqueueState::Queued);

    let repository = archive.memory_repository();
    let source = repository
        .get_source(source_id)
        .await?
        .expect("source remains active");
    assert_eq!(source.parts.len(), 2);
    assert_eq!(source.parts[0].role, SourcePartRole::User);
    assert_eq!(source.parts[1].role, SourcePartRole::Assistant);

    let lease = repository
        .claim_next_job()
        .await?
        .expect("source extraction is queued");
    let quote = "I prefer tea in the morning.";
    assert_eq!(
        repository
            .commit_extraction(
                lease,
                vec![NewMemory {
                    text: quote.to_owned(),
                    kind: MemoryRecordKind::Preference,
                    attribution: MemoryAttribution::UserStatement,
                    evidence: vec![EvidenceInput {
                        part_index: 0,
                        start_byte: 0,
                        end_byte: quote.len() as u32,
                        quote: quote.to_owned(),
                    }],
                    correction: None,
                    correction_needs_review: false,
                }],
            )
            .await?,
        CommitDisposition::Committed
    );

    let memories = repository
        .list_memories(MemoryListFilter {
            query: Some("tea".to_owned()),
            status: Some(MemoryRecordStatus::Active),
            limit: 10,
        })
        .await?;
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].attribution, MemoryAttribution::UserStatement);
    assert_eq!(memories[0].source_id, source_id);
    let hydrated = repository.hydrate_active(&[memories[0].id]).await?;
    assert_eq!(
        hydrated[0].source_created_at_unix_ms,
        source.created_at_unix_ms
    );
    assert_eq!(
        hydrated[0].evidence[0].source_created_at_unix_ms,
        source.created_at_unix_ms
    );
    assert_eq!(hydrated[0].evidence[0].part_index, 0);

    let forgotten = repository.forget_source(source_id).await?;
    assert_eq!(forgotten.suppressed_sources, vec![source_id]);
    assert_eq!(forgotten.affected_turn_ids, vec![receipt.turn_id]);
    assert!(
        repository
            .hydrate_active(&[memories[0].id])
            .await?
            .is_empty()
    );

    let projection = repository.claim_projection_page(10).await?;
    assert!(projection.items.iter().any(|item| {
        item.id == memories[0].id && item.operation == super::ProjectionOperation::Delete
    }));
    let deletion = projection
        .items
        .into_iter()
        .find(|item| item.id == memories[0].id)
        .expect("forgotten record has a pending delete");
    assert!(
        repository
            .finish_projection(deletion, ProjectionOutcome::Applied)
            .await?
    );

    archive.connection.clone().close().await?;
    drop(archive);
    remove_database(&path);
    Ok(())
}

#[tokio::test]
async fn import_revision_is_idempotent_and_unchanged_tombstones_stay_suppressed()
-> Result<(), Box<dyn Error>> {
    let path = database_path("import-revision");
    let archive = ConversationArchive::open(&path).await?;
    let repository = archive.memory_repository();
    let first = repository
        .import_source(
            "/notes/preferences.md".to_owned(),
            "preferences.md".to_owned(),
            "Use loose leaf tea.".to_owned(),
        )
        .await?;
    assert_eq!(first.state, SourceReceiptState::Created);
    let same = repository
        .import_source(
            "/notes/preferences.md".to_owned(),
            "preferences.md".to_owned(),
            "Use loose leaf tea.".to_owned(),
        )
        .await?;
    assert_eq!(same.state, SourceReceiptState::AlreadyCurrent);
    assert_eq!(same.source_id, first.source_id);

    let pending = repository.pending_source_parts(4).await?;
    let imported = pending
        .iter()
        .find(|pending| pending.source.id == first.source_id)
        .expect("import part is pending projection");
    let text = &imported.part.text;
    assert_eq!(
        repository
            .commit_source_chunks(
                first.source_id,
                first.revision_sha256.clone(),
                imported.part.index,
                vec![SourceChunk {
                    chunk_index: 0,
                    start_byte: 0,
                    end_byte: text.len() as u32,
                    text: text.clone(),
                }],
                true,
            )
            .await?,
        CommitDisposition::Committed
    );
    let memories = repository
        .list_memories(MemoryListFilter {
            query: Some("loose leaf".to_owned()),
            status: Some(MemoryRecordStatus::Active),
            limit: 10,
        })
        .await?;
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].attribution, MemoryAttribution::ImportedClaim);

    repository.forget_source(first.source_id).await?;
    let unchanged = repository
        .import_source(
            "/notes/preferences.md".to_owned(),
            "preferences.md".to_owned(),
            "Use loose leaf tea.".to_owned(),
        )
        .await?;
    assert_eq!(unchanged.state, SourceReceiptState::SuppressedRevision);
    assert!(
        repository
            .hydrate_active(&[memories[0].id])
            .await?
            .is_empty()
    );

    archive.connection.clone().close().await?;
    drop(repository);
    drop(archive);
    remove_database(&path);
    Ok(())
}

#[tokio::test]
async fn release_tracks_admission_and_forget_blocks_a_late_extraction() -> Result<(), Box<dyn Error>>
{
    let path = database_path("release-and-forget-race");
    let archive = ConversationArchive::open(&path).await?;
    let receipt = archive
        .append_turn_with_memory(
            "I prefer tea.",
            ArchiveOutcome::Completed("Understood.".to_owned()),
            MemoryEligibility::Eligible,
        )
        .await?;
    let source_id = receipt.source_id.expect("eligible turn has source");
    let repository = archive.memory_repository();

    let reserved = repository
        .claim_next_job()
        .await?
        .expect("queued source has a job");
    assert_eq!(reserved.attempt, 1);
    repository.release_job(reserved, false).await?;

    let admitted = repository
        .claim_next_job()
        .await?
        .expect("unadmitted reservation returns to queue");
    assert_eq!(admitted.attempt, 1);
    repository.release_job(admitted, true).await?;

    let stale = repository
        .claim_next_job()
        .await?
        .expect("admitted attempt remains consumed");
    assert_eq!(stale.attempt, 2);
    repository.forget_source(source_id).await?;
    let quote = "I prefer tea.";
    assert_eq!(
        repository
            .commit_extraction(
                stale,
                vec![NewMemory {
                    text: quote.to_owned(),
                    kind: MemoryRecordKind::Preference,
                    attribution: MemoryAttribution::UserStatement,
                    evidence: vec![EvidenceInput {
                        part_index: 0,
                        start_byte: 0,
                        end_byte: quote.len() as u32,
                        quote: quote.to_owned(),
                    }],
                    correction: None,
                    correction_needs_review: false,
                }],
            )
            .await?,
        CommitDisposition::LeaseLost
    );
    assert!(
        repository
            .list_memories(MemoryListFilter {
                query: None,
                status: Some(MemoryRecordStatus::Active),
                limit: 10,
            })
            .await?
            .is_empty()
    );

    archive.connection.clone().close().await?;
    drop(repository);
    drop(archive);
    remove_database(&path);
    Ok(())
}

#[tokio::test]
async fn archive_only_turn_is_not_added_by_historical_backfill() -> Result<(), Box<dyn Error>> {
    let path = database_path("archive-only-backfill");
    let archive = ConversationArchive::open(&path).await?;
    archive
        .append_turn(
            "Forget my previous preference.",
            ArchiveOutcome::Completed("I will forget it.".to_owned()),
        )
        .await?;

    let repository = archive.memory_repository();
    let page = repository.backfill_projection_page(8).await?;
    assert_eq!(page.examined_turns, 1);
    assert_eq!(page.created_sources, 0);
    assert!(page.complete);
    assert_eq!(repository.stats().await?.pending_sources, 0);

    archive.connection.clone().close().await?;
    drop(repository);
    drop(archive);
    remove_database(&path);
    Ok(())
}

#[tokio::test]
async fn forgetting_recalled_source_suppresses_answer_descendants_and_stale_projection()
-> Result<(), Box<dyn Error>> {
    let path = database_path("source-dependencies");
    let archive = ConversationArchive::open(&path).await?;
    let repository = archive.memory_repository();
    let original = archive
        .append_turn_with_memory(
            "I prefer tea.",
            ArchiveOutcome::Completed("I will remember that.".to_owned()),
            MemoryEligibility::Eligible,
        )
        .await?;
    let original_source = original.source_id.expect("eligible turn has a source");
    let original_lease = repository
        .claim_next_job()
        .await?
        .expect("original source queued");
    let quote = "I prefer tea.";
    assert_eq!(
        repository
            .commit_extraction(
                original_lease,
                vec![NewMemory {
                    text: quote.to_owned(),
                    kind: MemoryRecordKind::Preference,
                    attribution: MemoryAttribution::UserStatement,
                    evidence: vec![EvidenceInput {
                        part_index: 0,
                        start_byte: 0,
                        end_byte: quote.len() as u32,
                        quote: quote.to_owned(),
                    }],
                    correction: None,
                    correction_needs_review: false,
                }],
            )
            .await?,
        CommitDisposition::Committed
    );

    let answer = "You said you prefer tea.";
    let descendant = archive
        .append_turn_with_memory_and_sources(
            "What did I say I prefer?",
            ArchiveOutcome::Completed(answer.to_owned()),
            MemoryEligibility::Eligible,
            &[original_source],
        )
        .await?;
    let descendant_source = descendant.source_id.expect("recalled answer has source");
    let material = repository
        .get_source(descendant_source)
        .await?
        .expect("descendant source is initially active");
    assert_eq!(
        repository
            .commit_source_chunks(
                descendant_source,
                material.revision_sha256.clone(),
                1,
                vec![SourceChunk {
                    chunk_index: 0,
                    start_byte: 0,
                    end_byte: answer.len() as u32,
                    text: answer.to_owned(),
                }],
                true,
            )
            .await?,
        CommitDisposition::Committed
    );
    let descendant_memory = repository
        .list_memories(MemoryListFilter {
            query: Some("prefer tea".to_owned()),
            status: Some(MemoryRecordStatus::Active),
            limit: 10,
        })
        .await?
        .into_iter()
        .find(|memory| memory.source_id == descendant_source)
        .expect("assistant answer is separately attributed");
    assert_eq!(
        descendant_memory.attribution,
        MemoryAttribution::AssistantAnswer
    );
    let descendant_lease = repository
        .claim_next_job()
        .await?
        .expect("descendant extraction is queued");
    let stale_upsert = repository
        .claim_projection_page(10)
        .await?
        .items
        .into_iter()
        .find(|item| item.id == descendant_memory.id)
        .expect("assistant excerpt has a pending projection");

    let receipt = repository.forget_source(original_source).await?;
    assert_eq!(receipt.suppressed_source_count, 2);
    assert_eq!(receipt.affected_turn_count, 2);
    assert!(receipt.suppressed_sources.contains(&descendant_source));
    assert!(
        repository
            .hydrate_active(&[descendant_memory.id])
            .await?
            .is_empty()
    );
    assert_eq!(
        repository
            .commit_extraction(descendant_lease, Vec::new())
            .await?,
        CommitDisposition::LeaseLost
    );
    assert!(
        !repository
            .finish_projection(stale_upsert, ProjectionOutcome::Applied)
            .await?
    );
    let backfill = repository.backfill_projection_page(8).await?;
    assert_eq!(backfill.created_sources, 0);
    assert!(backfill.complete);

    let deletes = repository.claim_projection_page(10).await?;
    let descendant_delete = deletes
        .items
        .into_iter()
        .find(|item| {
            item.id == descendant_memory.id && item.operation == super::ProjectionOperation::Delete
        })
        .expect("forget schedules a descendant delete");
    assert!(
        repository
            .finish_projection(descendant_delete, ProjectionOutcome::Applied)
            .await?
    );

    archive.connection.clone().close().await?;
    drop(repository);
    drop(archive);
    remove_database(&path);
    Ok(())
}

#[tokio::test]
async fn corrections_require_one_grounded_local_user_fact_and_protect_excerpt_projections()
-> Result<(), Box<dyn Error>> {
    let path = database_path("grounded-corrections");
    let archive = ConversationArchive::open(&path).await?;
    let repository = archive.memory_repository();

    let original_quote = "My favourite drink is coffee.";
    let (original_source, original) = commit_user_memory(
        &archive,
        original_quote,
        MemoryRecordKind::Preference,
        None,
        false,
    )
    .await?;
    let original_material = repository
        .get_source(original_source)
        .await?
        .expect("original source is available");
    repository
        .commit_source_chunks(
            original_source,
            original_material.revision_sha256.clone(),
            0,
            vec![SourceChunk {
                chunk_index: 0,
                start_byte: 0,
                end_byte: original_quote.len() as u32,
                text: original_quote.to_owned(),
            }],
            true,
        )
        .await?;
    let original_excerpt = repository
        .list_memories(MemoryListFilter {
            query: None,
            status: Some(MemoryRecordStatus::Active),
            limit: 50,
        })
        .await?
        .into_iter()
        .find(|memory| {
            memory.source_id == original_source && memory.kind == MemoryRecordKind::SourceExcerpt
        })
        .expect("source chunk is projected as an excerpt");
    let stale_excerpt = repository
        .claim_projection_page(16)
        .await?
        .items
        .into_iter()
        .find(|lease| lease.id == original_excerpt.id)
        .expect("excerpt has a pending upsert");

    let correction_quote = "Actually my favourite drink is tea instead of coffee.";
    let (corrected_source, corrected) = commit_user_memory(
        &archive,
        correction_quote,
        MemoryRecordKind::Preference,
        Some(correction_evidence(
            correction_quote,
            CorrectionMarker::Actually,
            "my",
            "favourite drink",
            "coffee",
            "tea",
        )),
        false,
    )
    .await?;
    assert_ne!(corrected_source, original_source);
    assert_eq!(corrected.correction_state, CorrectionState::Applied);
    assert_eq!(corrected.supersedes_id, Some(original.id));
    assert!(
        repository
            .list_memories(MemoryListFilter {
                query: None,
                status: Some(MemoryRecordStatus::Superseded),
                limit: 50,
            })
            .await?
            .iter()
            .any(|memory| memory.id == original.id)
    );
    assert!(
        !repository
            .finish_projection(stale_excerpt, ProjectionOutcome::Applied)
            .await?
    );

    // A different chunk boundary arriving after correction must not recreate
    // an active excerpt covering the superseded source span.
    let late_end = "My favourite drink".len() as u32;
    repository
        .commit_source_chunks(
            original_source,
            original_material.revision_sha256,
            0,
            vec![SourceChunk {
                chunk_index: 1,
                start_byte: 0,
                end_byte: late_end,
                text: original_quote[..late_end as usize].to_owned(),
            }],
            true,
        )
        .await?;
    assert!(
        repository
            .list_memories(MemoryListFilter {
                query: None,
                status: Some(MemoryRecordStatus::Active),
                limit: 50,
            })
            .await?
            .iter()
            .all(|memory| memory.source_id != original_source
                || memory.kind != MemoryRecordKind::SourceExcerpt)
    );

    let water_quote = "Actually my favourite drink is water instead of tea.";
    let (_, water) = commit_user_memory(
        &archive,
        water_quote,
        MemoryRecordKind::Preference,
        Some(correction_evidence(
            water_quote,
            CorrectionMarker::Actually,
            "my",
            "favourite drink",
            "tea",
            "water",
        )),
        false,
    )
    .await?;
    assert_eq!(water.correction_state, CorrectionState::Applied);
    assert_eq!(water.supersedes_id, Some(corrected.id));

    // A later stale quote that mentions coffee as the rejected value cannot
    // replace the current water assertion.
    let stale_quote = "Actually my favourite drink is tea instead of coffee.";
    let (_, stale) = commit_user_memory(
        &archive,
        stale_quote,
        MemoryRecordKind::Preference,
        Some(correction_evidence(
            stale_quote,
            CorrectionMarker::Actually,
            "my",
            "favourite drink",
            "coffee",
            "tea",
        )),
        false,
    )
    .await?;
    assert_eq!(stale.correction_state, CorrectionState::NeedsReview);
    assert!(
        repository
            .list_memories(MemoryListFilter {
                query: None,
                status: Some(MemoryRecordStatus::Active),
                limit: 50,
            })
            .await?
            .iter()
            .any(|memory| memory.id == water.id)
    );

    let color_quote = "My favourite color is blue.";
    commit_user_memory(
        &archive,
        color_quote,
        MemoryRecordKind::Preference,
        None,
        false,
    )
    .await?;
    commit_user_memory(
        &archive,
        color_quote,
        MemoryRecordKind::Preference,
        None,
        false,
    )
    .await?;
    let ambiguous_quote = "Actually my favourite color is green instead of blue.";
    let (_, ambiguous) = commit_user_memory(
        &archive,
        ambiguous_quote,
        MemoryRecordKind::Preference,
        Some(correction_evidence(
            ambiguous_quote,
            CorrectionMarker::Actually,
            "my",
            "favourite color",
            "blue",
            "green",
        )),
        false,
    )
    .await?;
    assert_eq!(ambiguous.correction_state, CorrectionState::NeedsReview);
    assert_eq!(ambiguous.supersedes_id, None);

    let snack_quote = "My favourite snack is apples.";
    let (_, snack) = commit_user_memory(
        &archive,
        snack_quote,
        MemoryRecordKind::Preference,
        None,
        false,
    )
    .await?;
    let flagged_quote = "Actually my favourite snack is bananas instead of apples.";
    let (_, flagged) = commit_user_memory(
        &archive,
        flagged_quote,
        MemoryRecordKind::Preference,
        Some(correction_evidence(
            flagged_quote,
            CorrectionMarker::Actually,
            "my",
            "favourite snack",
            "apples",
            "bananas",
        )),
        true,
    )
    .await?;
    assert_eq!(flagged.correction_state, CorrectionState::NeedsReview);
    assert_eq!(flagged.supersedes_id, None);
    assert!(
        repository
            .list_memories(MemoryListFilter {
                query: None,
                status: Some(MemoryRecordStatus::Active),
                limit: 50,
            })
            .await?
            .iter()
            .any(|memory| memory.id == snack.id)
    );

    let underspecified_quote = "I may have changed my mind about that.";
    let (_, underspecified) = commit_user_memory(
        &archive,
        underspecified_quote,
        MemoryRecordKind::Preference,
        None,
        true,
    )
    .await?;
    assert_eq!(
        underspecified.correction_state,
        CorrectionState::NeedsReview
    );
    assert_eq!(underspecified.supersedes_id, None);

    let imported_text = "My favourite drink is cocoa.";
    let imported = repository
        .import_source(
            "preferences.md".to_owned(),
            "preferences.md".to_owned(),
            imported_text.to_owned(),
        )
        .await?;
    repository
        .commit_source_chunks(
            imported.source_id,
            imported.revision_sha256,
            0,
            vec![SourceChunk {
                chunk_index: 0,
                start_byte: 0,
                end_byte: imported_text.len() as u32,
                text: imported_text.to_owned(),
            }],
            true,
        )
        .await?;
    let imported_claim = repository
        .list_memories(MemoryListFilter {
            query: None,
            status: Some(MemoryRecordStatus::Active),
            limit: 50,
        })
        .await?
        .into_iter()
        .find(|memory| memory.source_id == imported.source_id)
        .expect("import chunk remains attributed and searchable");
    assert_eq!(imported_claim.attribution, MemoryAttribution::ImportedClaim);
    let imported_correction_quote = "Actually my favourite drink is tea instead of cocoa.";
    let (_, imported_correction) = commit_user_memory(
        &archive,
        imported_correction_quote,
        MemoryRecordKind::ImportedChunk,
        Some(correction_evidence(
            imported_correction_quote,
            CorrectionMarker::Actually,
            "my",
            "favourite drink",
            "cocoa",
            "tea",
        )),
        false,
    )
    .await?;
    assert_eq!(
        imported_correction.correction_state,
        CorrectionState::NeedsReview
    );
    assert_eq!(
        repository
            .hydrate_active(&[imported_claim.id])
            .await?
            .first()
            .expect("imported claim stays active")
            .attribution,
        MemoryAttribution::ImportedClaim
    );

    let meal_quote = "My favourite meal is noodles.";
    let (_, meal) = commit_user_memory(
        &archive,
        meal_quote,
        MemoryRecordKind::Preference,
        None,
        false,
    )
    .await?;
    let quoted_correction_quote = "Alice said, \"My favourite meal is rice instead of noodles.\"";
    let (_, quoted_correction) = commit_user_memory(
        &archive,
        quoted_correction_quote,
        MemoryRecordKind::Preference,
        Some(correction_evidence(
            quoted_correction_quote,
            CorrectionMarker::InsteadOf,
            "My",
            "favourite meal",
            "noodles",
            "rice",
        )),
        false,
    )
    .await?;
    assert_eq!(
        quoted_correction.correction_state,
        CorrectionState::NeedsReview
    );
    assert!(
        repository
            .list_memories(MemoryListFilter {
                query: None,
                status: Some(MemoryRecordStatus::Active),
                limit: 50,
            })
            .await?
            .iter()
            .any(|memory| memory.id == meal.id)
    );

    let narrative_prior_quote = "Nick said I prefer coffee.";
    let (_, narrative_prior) = commit_user_memory(
        &archive,
        narrative_prior_quote,
        MemoryRecordKind::Preference,
        None,
        false,
    )
    .await?;
    let direct_correction = "Actually I prefer tea instead of coffee.";
    let (_, narrative_prior_correction) = commit_user_memory(
        &archive,
        direct_correction,
        MemoryRecordKind::Preference,
        Some(correction_evidence(
            direct_correction,
            CorrectionMarker::Actually,
            "I",
            "prefer",
            "coffee",
            "tea",
        )),
        false,
    )
    .await?;
    assert_eq!(
        narrative_prior_correction.correction_state,
        CorrectionState::NeedsReview
    );
    assert!(
        repository
            .list_memories(MemoryListFilter {
                query: None,
                status: Some(MemoryRecordStatus::Active),
                limit: 50,
            })
            .await?
            .iter()
            .any(|memory| memory.id == narrative_prior.id)
    );

    let coffee_quote = "I prefer coffee.";
    let (_, coffee) = commit_user_memory(
        &archive,
        coffee_quote,
        MemoryRecordKind::Preference,
        None,
        false,
    )
    .await?;
    let unquoted_narrative = "Nick said I prefer tea instead of coffee.";
    let (_, unquoted) = commit_user_memory(
        &archive,
        unquoted_narrative,
        MemoryRecordKind::Preference,
        Some(correction_evidence(
            unquoted_narrative,
            CorrectionMarker::InsteadOf,
            "I",
            "prefer",
            "coffee",
            "tea",
        )),
        false,
    )
    .await?;
    assert_eq!(unquoted.correction_state, CorrectionState::NeedsReview);

    let single_quoted_narrative = "Nick said '\nI prefer tea instead of coffee.'";
    let (_, single_quoted) = commit_user_memory(
        &archive,
        single_quoted_narrative,
        MemoryRecordKind::Preference,
        Some(correction_evidence(
            single_quoted_narrative,
            CorrectionMarker::InsteadOf,
            "I",
            "prefer",
            "coffee",
            "tea",
        )),
        false,
    )
    .await?;
    assert_eq!(single_quoted.correction_state, CorrectionState::NeedsReview);
    assert!(
        repository
            .list_memories(MemoryListFilter {
                query: None,
                status: Some(MemoryRecordStatus::Active),
                limit: 50,
            })
            .await?
            .iter()
            .any(|memory| memory.id == coffee.id)
    );

    archive.connection.clone().close().await?;
    drop(repository);
    drop(archive);
    remove_database(&path);
    Ok(())
}

#[tokio::test]
async fn migration_from_v1_preserves_archive_rows_and_rejects_future_schema()
-> Result<(), Box<dyn Error>> {
    let path = database_path("migration");
    let connection = rusqlite::Connection::open(&path)?;
    connection.execute_batch(
        "CREATE TABLE sessions (id INTEGER PRIMARY KEY, started_at_unix_ms INTEGER NOT NULL);
         CREATE TABLE turns (
             id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES sessions(id),
             created_at_unix_ms INTEGER NOT NULL, user_text TEXT NOT NULL, assistant_text TEXT,
             outcome TEXT NOT NULL
         );
         INSERT INTO sessions VALUES (1, 10);
         INSERT INTO turns VALUES (1, 1, 11, 'old prompt', 'old answer', 'completed');
         PRAGMA user_version = 1;",
    )?;
    drop(connection);

    let archive = ConversationArchive::open(&path).await?;
    let version: i64 = archive
        .connection
        .call(|connection| connection.query_row("PRAGMA user_version", [], |row| row.get(0)))
        .await?;
    assert_eq!(version, 3);
    let row: (i64, String, String) = archive
        .connection
        .call(|connection| {
            connection.query_row(
                "SELECT created_at_unix_ms, user_text, assistant_text FROM turns WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
        })
        .await?;
    assert_eq!(row, (11, "old prompt".to_owned(), "old answer".to_owned()));
    let backfill = archive
        .memory_repository()
        .backfill_projection_page(8)
        .await?;
    assert_eq!(backfill.created_sources, 1);
    let source_created_at: i64 = archive
        .connection
        .call(|connection| {
            connection.query_row(
                "SELECT created_at_unix_ms FROM memory_sources WHERE turn_id = 1",
                [],
                |row| row.get(0),
            )
        })
        .await?;
    assert_eq!(source_created_at, 11);
    let stats = archive.memory_repository().stats().await?;
    assert_eq!(stats.queued_extractions, 0);
    assert_eq!(stats.pending_sources, 0);

    archive.connection.clone().close().await?;
    drop(archive);
    let connection = rusqlite::Connection::open(&path)?;
    connection.pragma_update(None, "user_version", 4)?;
    drop(connection);
    assert!(matches!(
        ConversationArchive::open(&path).await,
        Err(StorageError::UnsupportedSchemaVersion(4))
    ));
    remove_database(&path);
    Ok(())
}
