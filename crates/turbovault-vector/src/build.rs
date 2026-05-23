use crate::{
    EmbeddingEngine, VectorError, VectorIndex,
    chunks::{Chunk, ChunkStore, chunk_text},
};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::UNIX_EPOCH;
use tracing::{info, warn};
use turbovault_parser::to_plain_text;

fn sha256_hex(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// Builds and updates the HNSW index from vault content.
pub struct IndexBuilder {
    embedder: Arc<dyn EmbeddingEngine>,
    chunks: Arc<ChunkStore>,
    chunk_max_chars: usize,
    chunk_overlap_chars: usize,
}

#[derive(Debug, Default)]
pub struct RebuildStats {
    pub notes_indexed: usize,
    pub chunks_created: usize,
    pub elapsed_ms: u64,
    pub model: String,
}

impl IndexBuilder {
    pub fn new(
        embedder: Arc<dyn EmbeddingEngine>,
        chunks: Arc<ChunkStore>,
        chunk_max_chars: usize,
        chunk_overlap_chars: usize,
    ) -> Self {
        Self {
            embedder,
            chunks,
            chunk_max_chars,
            chunk_overlap_chars,
        }
    }

    pub async fn full_rebuild(
        &self,
        vault_root: &Path,
        index: &mut VectorIndex,
    ) -> Result<RebuildStats, VectorError> {
        let start = std::time::Instant::now();

        self.chunks.clear_all()?;

        // Collect all .md files
        let md_files: Vec<std::path::PathBuf> = walkdir::WalkDir::new(vault_root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension() == Some("md".as_ref()))
            .map(|e| e.path().to_path_buf())
            .collect();

        let total = md_files.len();
        self.chunks
            .set_model_meta(self.embedder.model_name(), self.embedder.dimensions())?;

        let mut notes_indexed = 0usize;
        let mut chunks_created = 0usize;

        // Process in batches of 32 files
        for batch in md_files.chunks(32) {
            for file_path in batch {
                let raw_content = match tokio::fs::read_to_string(&file_path).await {
                    Ok(c) => c,
                    Err(e) => {
                        warn!("Failed to read {}: {}", file_path.display(), e);
                        continue;
                    }
                };

                let mtime = match std::fs::metadata(file_path)
                    .and_then(|m| m.modified())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).map_err(std::io::Error::other))
                {
                    Ok(d) => d.as_millis() as i64,
                    Err(e) => {
                        warn!("Failed to get mtime for {}: {}", file_path.display(), e);
                        0
                    }
                };

                let plain = to_plain_text(&raw_content);
                let file_hash = sha256_hex(plain.as_bytes());

                let rel_path = file_path
                    .strip_prefix(vault_root)
                    .unwrap_or(file_path)
                    .to_string_lossy()
                    .to_string();

                let ranges = chunk_text(&plain, self.chunk_max_chars, self.chunk_overlap_chars);
                if ranges.is_empty() {
                    // Empty file: record it so incremental updates work, but no chunks.
                    self.chunks.upsert_file(&rel_path, mtime, &file_hash)?;
                    notes_indexed += 1;
                    continue;
                }

                // Collect chunk texts for this file
                let chunk_texts: Vec<&str> = ranges
                    .iter()
                    .map(|(start, end)| &plain[*start..*end])
                    .collect();

                // Embed all chunks for this file in one call
                let embeddings = match self.embedder.embed(&chunk_texts).await {
                    Ok(e) => e,
                    Err(e) => {
                        warn!("Failed to embed chunks for {}: {}", rel_path, e);
                        notes_indexed += 1;
                        continue;
                    }
                };

                // Allocate IDs for this file's chunks with a single DB read (optimization 2).
                let mut chunk_id = self.chunks.next_chunk_id()?;
                let total_chunks = ranges.len() as u32;
                let mut chunk_batch: Vec<Chunk> = Vec::with_capacity(ranges.len());

                for (chunk_index, (start, end)) in ranges.iter().enumerate() {
                    let chunk_text_slice = &plain[*start..*end];
                    chunk_batch.push(Chunk {
                        id: chunk_id,
                        note_path: rel_path.clone(),
                        chunk_index: chunk_index as u32,
                        total_chunks,
                        start_byte: *start as u64,
                        end_byte: *end as u64,
                        content_hash: sha256_hex(chunk_text_slice.as_bytes()),
                        preview: chunk_text_slice.chars().take(120).collect(),
                    });
                    chunk_id += 1;
                }

                // Single transaction: upsert file record + all chunk rows (optimization 1).
                self.chunks
                    .insert_chunks_tx(&rel_path, mtime, &file_hash, &chunk_batch)?;

                // HNSW upserts happen outside the DB transaction.
                for (chunk, vector) in chunk_batch.iter().zip(embeddings.iter()) {
                    index.upsert(chunk.id, vector)?;
                }
                chunks_created += chunk_batch.len();

                notes_indexed += 1;

                if notes_indexed.is_multiple_of(100) {
                    info!("Indexed {}/{} files", notes_indexed, total);
                }
            }
        }

        index.flush()?;

        let elapsed_ms = start.elapsed().as_millis() as u64;

        Ok(RebuildStats {
            notes_indexed,
            chunks_created,
            elapsed_ms,
            model: self.embedder.model_name().to_string(),
        })
    }

    pub async fn update_file(
        &self,
        file_path: &Path,
        vault_root: &Path,
        index: &mut VectorIndex,
    ) -> Result<(), VectorError> {
        let rel_path = file_path
            .strip_prefix(vault_root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();

        // Check if file exists; if not, treat as deletion.
        let mtime = match std::fs::metadata(file_path) {
            Ok(meta) => match meta.modified() {
                Ok(t) => match t.duration_since(UNIX_EPOCH) {
                    Ok(d) => d.as_millis() as i64,
                    Err(_) => 0,
                },
                Err(_) => 0,
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let old_ids = self.chunks.delete_chunks_for_file(&rel_path)?;
                let had_chunks = !old_ids.is_empty();
                for id in old_ids {
                    if let Err(e) = index.remove(id) {
                        warn!("Failed to remove vector for chunk {}: {}", id, e);
                    }
                }
                if had_chunks {
                    index.flush()?;
                }
                return Ok(());
            }
            Err(e) => return Err(VectorError::Io(e)),
        };

        // Re-read and compute content hash.
        let raw_content = tokio::fs::read_to_string(file_path).await?;
        let plain = to_plain_text(&raw_content);
        let file_hash = sha256_hex(plain.as_bytes());

        // Fast path: content unchanged → just update mtime.
        if let Some((stored_mtime, stored_hash)) = self.chunks.get_file_info(&rel_path)?
            && stored_hash == file_hash
        {
            if stored_mtime != mtime {
                self.chunks.upsert_file(&rel_path, mtime, &file_hash)?;
            }
            return Ok(());
        }

        let ranges = chunk_text(&plain, self.chunk_max_chars, self.chunk_overlap_chars);

        // Empty file: remove all stale chunks.
        if ranges.is_empty() {
            let old_ids = self.chunks.delete_chunks_for_file(&rel_path)?;
            let had_chunks = !old_ids.is_empty();
            for id in old_ids {
                if let Err(e) = index.remove(id) {
                    warn!("Failed to remove vector for chunk {}: {}", id, e);
                }
            }
            self.chunks.upsert_file(&rel_path, mtime, &file_hash)?;
            if had_chunks {
                index.flush()?;
            }
            return Ok(());
        }

        let chunk_texts: Vec<&str> = ranges.iter().map(|(s, e)| &plain[*s..*e]).collect();
        let new_hashes: Vec<String> = chunk_texts
            .iter()
            .map(|t| sha256_hex(t.as_bytes()))
            .collect();

        // Diff against stored chunks — lightweight query (optimization 3).
        let old_hashes = self.chunks.get_chunk_hashes_for_file(&rel_path)?;
        let (reuse_ids, ids_to_delete) = diff_chunks(&old_hashes, &new_hashes);

        // Remove stale vectors from HNSW before the DB transaction.
        for &id in &ids_to_delete {
            if let Err(e) = index.remove(id) {
                warn!("Failed to remove vector for chunk {}: {}", id, e);
            }
        }

        // Embed only chunks with no matching old vector.
        let embed_positions: Vec<usize> = reuse_ids
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.is_none().then_some(i))
            .collect();

        let new_embeddings = if embed_positions.is_empty() {
            vec![]
        } else {
            let texts: Vec<&str> = embed_positions.iter().map(|&i| chunk_texts[i]).collect();
            self.embedder.embed(&texts).await?
        };

        // Allocate IDs for new chunks with a single DB read (optimization 2).
        let new_count = embed_positions.len();
        let mut next_id = if new_count > 0 {
            self.chunks.next_chunk_id()?
        } else {
            0
        };

        // Build the full chunk list.
        let total_chunks = ranges.len() as u32;
        let mut vector_changed = !ids_to_delete.is_empty();
        let mut chunk_batch: Vec<Chunk> = Vec::with_capacity(ranges.len());

        for (i, ((start, end), (hash, reuse_id))) in ranges
            .iter()
            .zip(new_hashes.iter().zip(reuse_ids.iter()))
            .enumerate()
        {
            let chunk_id = match reuse_id {
                Some(id) => *id,
                None => {
                    let id = next_id;
                    next_id += 1;
                    vector_changed = true;
                    id
                }
            };
            chunk_batch.push(Chunk {
                id: chunk_id,
                note_path: rel_path.clone(),
                chunk_index: i as u32,
                total_chunks,
                start_byte: *start as u64,
                end_byte: *end as u64,
                content_hash: hash.clone(),
                preview: chunk_texts[i].chars().take(120).collect(),
            });
        }

        // Single transaction: delete stale + insert all chunks + update file record (optimization 1).
        self.chunks.update_file_chunks_tx(
            &rel_path,
            mtime,
            &file_hash,
            &ids_to_delete,
            &chunk_batch,
        )?;

        // HNSW upserts for new chunks (outside the DB transaction).
        let mut embed_idx = 0;
        for (chunk, reuse_id) in chunk_batch.iter().zip(reuse_ids.iter()) {
            if reuse_id.is_none() {
                index.upsert(chunk.id, &new_embeddings[embed_idx])?;
                embed_idx += 1;
            }
        }

        if vector_changed {
            index.flush()?;
        }
        Ok(())
    }
}

/// Match new chunk hashes against stored chunks by content hash.
///
/// Returns `(reuse_ids, ids_to_delete)` where:
/// - `reuse_ids[i] = Some(id)` — new chunk i can reuse an existing vector; no re-embedding needed.
/// - `reuse_ids[i] = None`    — new chunk i is new or changed; must be embedded.
/// - `ids_to_delete`          — old chunk IDs not matched by any new chunk; stale, remove from index.
///
/// Duplicate content (same hash appearing multiple times) is handled via a per-hash queue:
/// each old ID is consumed at most once, preserving as many vectors as possible.
fn diff_chunks(
    old_hashes: &[(u64, String)],
    new_hashes: &[String],
) -> (Vec<Option<u64>>, Vec<u64>) {
    let mut old_by_hash: HashMap<&str, VecDeque<u64>> = HashMap::new();
    for (id, hash) in old_hashes {
        old_by_hash.entry(hash.as_str()).or_default().push_back(*id);
    }

    let reuse_ids: Vec<Option<u64>> = new_hashes
        .iter()
        .map(|hash| {
            old_by_hash
                .get_mut(hash.as_str())
                .and_then(|q| q.pop_front())
        })
        .collect();

    let ids_to_delete: Vec<u64> = old_by_hash.into_values().flatten().collect();

    (reuse_ids, ids_to_delete)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn old(v: &[(u64, &str)]) -> Vec<(u64, String)> {
        v.iter().map(|(id, h)| (*id, h.to_string())).collect()
    }

    fn hashes(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // ── diff_chunks unit tests ────────────────────────────────────────────────

    #[test]
    fn diff_empty_old_and_new() {
        let (reuse, delete) = diff_chunks(&[], &[]);
        assert!(reuse.is_empty());
        assert!(delete.is_empty());
    }

    #[test]
    fn diff_all_new_no_old() {
        let (reuse, delete) = diff_chunks(&[], &hashes(&["a", "b", "c"]));
        assert_eq!(reuse, vec![None, None, None]);
        assert!(delete.is_empty());
    }

    #[test]
    fn diff_all_unchanged() {
        let (reuse, delete) = diff_chunks(
            &old(&[(1, "a"), (2, "b"), (3, "c")]),
            &hashes(&["a", "b", "c"]),
        );
        assert_eq!(reuse, vec![Some(1), Some(2), Some(3)]);
        assert!(delete.is_empty());
    }

    #[test]
    fn diff_one_chunk_changed() {
        // chunk index 1 changed from "b" to "b2"
        let (reuse, mut delete) = diff_chunks(
            &old(&[(1, "a"), (2, "b"), (3, "c")]),
            &hashes(&["a", "b2", "c"]),
        );
        assert_eq!(reuse, vec![Some(1), None, Some(3)]);
        delete.sort_unstable();
        assert_eq!(delete, vec![2]);
    }

    #[test]
    fn diff_paragraph_inserted_middle() {
        // "new" inserted between "a" and "b"; "a" and "b" content is the same
        let (reuse, delete) = diff_chunks(&old(&[(1, "a"), (2, "b")]), &hashes(&["a", "new", "b"]));
        assert_eq!(reuse, vec![Some(1), None, Some(2)]);
        assert!(delete.is_empty());
    }

    #[test]
    fn diff_paragraph_deleted() {
        // middle chunk removed
        let (reuse, mut delete) =
            diff_chunks(&old(&[(1, "a"), (2, "b"), (3, "c")]), &hashes(&["a", "c"]));
        assert_eq!(reuse, vec![Some(1), Some(3)]);
        delete.sort_unstable();
        assert_eq!(delete, vec![2]);
    }

    #[test]
    fn diff_all_deleted() {
        let (reuse, mut delete) = diff_chunks(&old(&[(1, "a"), (2, "b")]), &[]);
        assert!(reuse.is_empty());
        delete.sort_unstable();
        assert_eq!(delete, vec![1, 2]);
    }

    #[test]
    fn diff_duplicate_content_partial_match() {
        // Two identical chunks in old; new has only one → one reused, one deleted.
        let (reuse, mut delete) = diff_chunks(&old(&[(10, "dup"), (11, "dup")]), &hashes(&["dup"]));
        // One of {10, 11} is reused; the other is deleted.
        assert_eq!(reuse.len(), 1);
        assert!(reuse[0].is_some());
        delete.sort_unstable();
        assert_eq!(delete.len(), 1);
        let reused_id = reuse[0].unwrap();
        assert!(!delete.contains(&reused_id));
    }

    #[test]
    fn diff_duplicate_content_full_match() {
        // Two identical chunks in both old and new → both reused, nothing deleted.
        let (reuse, delete) =
            diff_chunks(&old(&[(10, "dup"), (11, "dup")]), &hashes(&["dup", "dup"]));
        assert_eq!(reuse.len(), 2);
        assert!(reuse.iter().all(|r| r.is_some()));
        assert!(delete.is_empty());
    }

    // ── update_file integration tests ────────────────────────────────────────

    #[cfg(feature = "local")]
    mod integration {
        use super::super::*;
        use crate::{ChunkStore, VectorIndex};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use tempfile::TempDir;
        use tokio::time::Duration;

        /// Embedder that counts how many texts were embedded and returns
        /// unit vectors of the requested dimensionality.
        struct CountingEmbedder {
            count: Arc<AtomicUsize>,
            dims: usize,
        }

        #[async_trait::async_trait]
        impl EmbeddingEngine for CountingEmbedder {
            async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VectorError> {
                self.count.fetch_add(texts.len(), Ordering::SeqCst);
                // Deterministic non-zero unit-ish vectors (avoids zero-norm issues).
                Ok(texts
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        let mut v = vec![0.0f32; self.dims];
                        v[i % self.dims] = 1.0;
                        v
                    })
                    .collect())
            }
            fn dimensions(&self) -> usize {
                self.dims
            }
            fn model_name(&self) -> &str {
                "counting-test-embedder"
            }
        }

        const DIMS: usize = 8;

        fn setup(db_dir: &TempDir) -> (Arc<ChunkStore>, Arc<CountingEmbedder>, Arc<AtomicUsize>) {
            let chunks = Arc::new(ChunkStore::open(&db_dir.path().join("state.db")).unwrap());
            let count = Arc::new(AtomicUsize::new(0));
            let embedder = Arc::new(CountingEmbedder {
                count: count.clone(),
                dims: DIMS,
            });
            (chunks, embedder, count)
        }

        fn make_builder(embedder: Arc<CountingEmbedder>, chunks: Arc<ChunkStore>) -> IndexBuilder {
            // chunk_max_chars=30 fits each 26-27 char test sentence as exactly one chunk.
            // overlap=0 so adjacent chunks don't share content and won't be invalidated
            // by a neighbour's change.
            IndexBuilder::new(embedder, chunks, 30, 0)
        }

        fn open_index(db_dir: &TempDir) -> VectorIndex {
            VectorIndex::open_or_create(&db_dir.path().join("hnsw.idx"), DIMS, "f32").unwrap()
        }

        /// Write `content` to `path` and wait long enough for the filesystem
        /// mtime to advance (NTFS 100 ns resolution; 50 ms is more than enough).
        async fn write_and_wait(path: &std::path::Path, content: &str) {
            tokio::fs::write(path, content).await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // Test content uses short sentences (≤27 chars each) separated by ". " so
        // the chunker (split-on-". ", max_chars=30, overlap=0) produces exactly one
        // chunk per sentence and edits to one sentence don't bleed into neighbours.
        //
        // to_plain_text joins blocks with "\n" (not "\n\n"), so paragraph breaks in
        // the source markdown collapse. Sentence-level splits are stable across that
        // transform and give us predictable chunk boundaries.
        const S1: &str = "Alpha unique text here."; // 23 chars
        const S2: &str = "Beta unique text here."; // 22 chars
        const S3: &str = "Gamma unique text here."; // 23 chars
        const S2_MOD: &str = "Beta changed text here."; // 23 chars — different hash from S2
        const S4: &str = "Delta unique text here."; // 23 chars — new sentence for append test

        fn three_sentences() -> String {
            format!("{S1} {S2} {S3}")
        }
        fn two_sentences() -> String {
            format!("{S1} {S2}")
        }

        #[tokio::test]
        async fn initial_index_embeds_all_chunks() {
            let vault = TempDir::new().unwrap();
            let db_dir = TempDir::new().unwrap();
            let (chunks, embedder, count) = setup(&db_dir);
            let builder = make_builder(embedder, chunks);
            let mut index = open_index(&db_dir);

            let note = vault.path().join("note.md");
            write_and_wait(&note, &three_sentences()).await;

            builder
                .update_file(&note, vault.path(), &mut index)
                .await
                .unwrap();
            assert_eq!(
                count.load(Ordering::SeqCst),
                3,
                "should embed all 3 chunks on first index"
            );
        }

        #[tokio::test]
        async fn unchanged_mtime_skips_all_embedding() {
            let vault = TempDir::new().unwrap();
            let db_dir = TempDir::new().unwrap();
            let (chunks, embedder, count) = setup(&db_dir);
            let builder = make_builder(embedder, chunks);
            let mut index = open_index(&db_dir);

            let note = vault.path().join("note.md");
            write_and_wait(&note, &two_sentences()).await;
            builder
                .update_file(&note, vault.path(), &mut index)
                .await
                .unwrap();
            let after_first = count.load(Ordering::SeqCst);

            // Second call with identical mtime → early exit, zero embedding.
            builder
                .update_file(&note, vault.path(), &mut index)
                .await
                .unwrap();
            assert_eq!(
                count.load(Ordering::SeqCst),
                after_first,
                "mtime unchanged: must embed nothing on second call"
            );
        }

        async fn check_incremental_update(
            initial_content: &str,
            updated_content: &str,
            expected_count: usize,
            msg: &str,
        ) {
            let vault = TempDir::new().unwrap();
            let db_dir = TempDir::new().unwrap();
            let (chunks, embedder, count) = setup(&db_dir);
            let builder = make_builder(embedder, chunks);
            let mut index = open_index(&db_dir);

            let note = vault.path().join("note.md");
            write_and_wait(&note, initial_content).await;
            builder
                .update_file(&note, vault.path(), &mut index)
                .await
                .unwrap();
            assert_eq!(count.load(Ordering::SeqCst), 3);

            write_and_wait(&note, updated_content).await;
            builder
                .update_file(&note, vault.path(), &mut index)
                .await
                .unwrap();
            assert_eq!(count.load(Ordering::SeqCst), expected_count, "{msg}");
        }

        #[tokio::test]
        async fn one_sentence_changed_reembeds_only_that_chunk() {
            check_incremental_update(
                &three_sentences(),
                &format!("{S1} {S2_MOD} {S3}"),
                4,
                "only the changed sentence-chunk should be re-embedded",
            )
            .await;
        }

        #[tokio::test]
        async fn appended_sentence_embeds_only_new_chunk() {
            check_incremental_update(
                &three_sentences(),
                &format!("{S1} {S2} {S3} {S4}"),
                4,
                "only the new appended chunk should be embedded",
            )
            .await;
        }

        #[tokio::test]
        async fn deleted_file_removes_chunks_from_index() {
            let vault = TempDir::new().unwrap();
            let db_dir = TempDir::new().unwrap();
            let (chunks, embedder, _count) = setup(&db_dir);
            let builder = make_builder(Arc::clone(&embedder), Arc::clone(&chunks));
            let mut index = open_index(&db_dir);

            let note = vault.path().join("note.md");
            write_and_wait(&note, &two_sentences()).await;
            builder
                .update_file(&note, vault.path(), &mut index)
                .await
                .unwrap();
            assert!(
                index.len() > 0,
                "index should have entries after initial index"
            );

            tokio::fs::remove_file(&note).await.unwrap();
            builder
                .update_file(&note, vault.path(), &mut index)
                .await
                .unwrap();
            assert_eq!(
                index.len(),
                0,
                "deleted file: all vectors should be removed"
            );

            let stored = chunks.get_chunks_for_file("note.md").unwrap();
            assert!(
                stored.is_empty(),
                "deleted file: chunk records should be removed from DB"
            );
        }

        #[tokio::test]
        async fn full_rewrite_reembeds_all_chunks() {
            let vault = TempDir::new().unwrap();
            let db_dir = TempDir::new().unwrap();
            let (chunks, embedder, count) = setup(&db_dir);
            let builder = make_builder(embedder, chunks);
            let mut index = open_index(&db_dir);

            let note = vault.path().join("note.md");
            write_and_wait(&note, &two_sentences()).await;
            builder
                .update_file(&note, vault.path(), &mut index)
                .await
                .unwrap();
            let initial = count.load(Ordering::SeqCst);
            assert!(initial > 0);

            // Completely different content — no hash should match.
            write_and_wait(&note, "Rewritten first thing. Rewritten second thing.").await;
            builder
                .update_file(&note, vault.path(), &mut index)
                .await
                .unwrap();
            let after = count.load(Ordering::SeqCst);
            assert_eq!(
                after - initial,
                initial,
                "full rewrite: must re-embed every chunk (none reused)"
            );
        }
    }
}
