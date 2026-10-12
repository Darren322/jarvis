# Memory and archive operations

## Where things live

| Path | Contents | Override |
| --- | --- | --- |
| `data/jarvis.sqlite3` | The only durable store: sessions, turns, memory sources, memories, the FTS5 keyword index and the sqlite-vec vector table | `JARVIS_ARCHIVE_PATH` |
| `data/embeddings/` | Local BGE ONNX model files (until the remote BGE server is integrated) | `JARVIS_EMBEDDING_MODEL_DIR` |

`data/memory-index/` belonged to the retired LanceDB index (schema v3 and earlier). Nothing reads it any more, so you can delete it once the v4 migration has run.

## How memory works (schema v4)

- **Sources.** Each completed, eligible turn and each imported file becomes a row in `sources`. A turn also records which earlier sources were recalled into its run (`source_links`).
- **Memories.** Sources are split into chunks of at most 1,500 bytes in `memories`. They're written in the same transaction as the archived turn.
- **Search.** Insert and delete triggers keep two indexes in sync: `memories_fts` (FTS5, porter stemming) and `memory_vectors` (sqlite-vec, 384-dimension BGE vectors). Recall fuses BM25 and cosine rankings with reciprocal rank fusion.
- **Embedding.** Vectors are added during idle time. `embedding_model` records which model produced each row's vector, so changing the model automatically queues everything for re-embedding. Until a row has a vector it can still be found by keyword.
- **Forgetting.** Forgetting a memory forgets its whole source and, transitively, every answer that recalled it. Their memory rows are deleted (the triggers clear both indexes) and the sources are marked `forgotten`. Archived turns stay in `turns`. A forgotten import revision isn't re-imported.

## Commands

| Command | Effect |
| --- | --- |
| `/memories [text]` | List the newest memories, optionally filtered by text |
| `/forget <id>`, `forget that`, `forget <detail>` | Forget a memory's source and everything built from it |
| `/import <path>` | Import a `.txt` or `.md` file (≤ 1 MiB); a changed file supersedes the old version |
| `/memory status` | Counts, pending embeddings, and embedder state |
| `/memory rebuild` | Drop all vectors; idle time re-embeds them |

## Upgrading from v3 or earlier

The first v4 start migrates in one transaction. If anything fails, nothing changes.

1. Stop Jarvis and make a backup: `sqlite3 data/jarvis.sqlite3 '.backup data/jarvis-pre-v4.sqlite3'`
2. Start Jarvis. The migration does the following:
   - re-chunks every active conversation turn, including turns the old backfill never reached
   - keeps imported chunks and LLM-extracted facts (marked `[fact]` in `/memories`)
   - keeps forgotten sources forgotten
   - preserves lineage links
   - drops the eight v3 memory tables
3. Run `/memory status` and watch "awaiting embedding" fall to zero while idle.
4. Delete `data/memory-index/`.

Downgrading isn't supported. Restore the backup instead.

## Backup

Exit Jarvis normally, then use SQLite's online backup rather than copying a live file:

```sh
sqlite3 data/jarvis.sqlite3 '.backup data/jarvis-backup.sqlite3'
```

The `sqlite3` CLI doesn't load sqlite-vec, so it can't query `memory_vectors`, but `.backup` copies the whole file correctly. A restored backup is complete: the indexes live inside the same file.
