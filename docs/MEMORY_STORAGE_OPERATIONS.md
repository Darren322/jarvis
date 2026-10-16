# Memory and archive operations

## Where things live

| Path | Contents | Override |
| --- | --- | --- |
| `data/jarvis.sqlite3` | The only durable store: sessions, turns, memory sources, memories, the FTS5 keyword index and the sqlite-vec vector table | `JARVIS_ARCHIVE_PATH` |
| BGE embedding server | `llama-server --embedding` on the 16 GB Pi, port 8081 ([setup](TWO_PI_BGE_DEPLOYMENT.md)) | `JARVIS_EMBEDDING_BASE_URL`, `JARVIS_EMBEDDING_MODEL` |

`data/memory-index/` belonged to the retired LanceDB index (schema v3 and earlier). Nothing reads it any more, so you can delete it once the v4 migration has run.

## How memory works (schema v4)

- **Sources.** Each completed, eligible turn and each imported file becomes a row in `sources`. A turn also records which earlier sources were recalled into its run (`source_links`).
- **Memories.** Sources are split into chunks of at most 1,500 bytes in `memories`. They're written in the same transaction as the archived turn.
- **Search.** Insert and delete triggers keep two indexes in sync: `memories_fts` (FTS5, porter stemming) and `memory_vectors` (sqlite-vec, 384-dimension BGE vectors). Recall fuses BM25 and cosine rankings with reciprocal rank fusion.
- **Embedding.** Vectors come from the BGE server, one request at a time, and are added during idle time. The server accepts at most 512 tokens, so longer text is embedded from its first 511 tokens. If the server is down, idle embedding pauses and recall uses keywords; the next successful question resumes both. `embedding_model` records which model produced each row's vector, so changing the model automatically queues everything for re-embedding. Until a row has a vector it can still be found by keyword.
- **Forgetting.** Forgetting a memory forgets its whole source and, transitively, every answer that recalled it. Their memory rows are deleted (the triggers clear both indexes) and the sources are marked `forgotten`. Archived turns stay in `turns`. A forgotten import revision isn't re-imported.

## Commands

| Command | Effect |
| --- | --- |
| `/memories [text]` | List the newest memories, optionally filtered by text |
| `/forget <id>`, `forget that`, `forget <detail>` | Forget a memory's source and everything built from it |
| `/import <path>` | Import a `.txt` or `.md` file (≤ 1 MiB); a changed file supersedes the old version |
| `/memory status` | Counts, pending embeddings, and embedder state (`connecting`, `ready`, or `unavailable`) |
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

## Switching the 8 GB Pi to the embedding server

Do this once, after the BGE server answers on the 16 GB Pi. If the Pi still has schema v3, do the v4 upgrade above in the same start.

1. Stop Jarvis and back up: `sqlite3 data/jarvis.sqlite3 '.backup data/jarvis-pre-bge.sqlite3'`
2. Build the new version. In `.env`, add `JARVIS_EMBEDDING_BASE_URL=http://jarvis-ai.local:8081/v1` and `JARVIS_EMBEDDING_MODEL=bge-small-en-v1.5`, and remove `JARVIS_EMBEDDING_MODEL_DIR` and `ORT_DYLIB_PATH`.
3. Start Jarvis and run `/memory status`. The embedder should be `ready`, and "awaiting embedding" should equal the number of memories: the model fingerprint changed, so every memory is re-embedded.
4. Leave Jarvis idle until "awaiting embedding" reaches 0. Ask Gemma something meanwhile and note whether replies are slower; both models share the 16 GB Pi.
5. Ask about something discussed earlier, worded differently. It should be recalled without a keyword-only warning.
6. Stop only the BGE server and ask again: you get a keyword-only warning and chat still works. Start the server again; the next question recovers without restarting Jarvis.
7. Delete `data/embeddings/` and the ONNX Runtime folder that `scripts/embeddings/setup.sh` created.

**Rollback:** restore the backup and run the commit before `3287032` with the old `.env` lines.

## Backup

Exit Jarvis normally, then use SQLite's online backup rather than copying a live file:

```sh
sqlite3 data/jarvis.sqlite3 '.backup data/jarvis-backup.sqlite3'
```

The `sqlite3` CLI doesn't load sqlite-vec, so it can't query `memory_vectors`, but `.backup` copies the whole file correctly. A restored backup is complete: the indexes live inside the same file.
