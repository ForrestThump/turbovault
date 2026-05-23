use crate::error::VectorError;
use std::path::Path;
use std::sync::Mutex;

#[cfg(feature = "local")]
use rusqlite::{Connection, params};

/// A single indexed chunk.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub id: u64,
    pub note_path: String,
    pub chunk_index: u32,
    pub total_chunks: u32,
    pub start_byte: u64,
    pub end_byte: u64,
    pub content_hash: String,
    pub preview: String,
}

#[cfg(feature = "local")]
const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS files (
    path         TEXT PRIMARY KEY,
    mtime        INTEGER NOT NULL,
    content_hash TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS paragraphs (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path    TEXT NOT NULL,
    para_index   INTEGER NOT NULL,
    start_byte   INTEGER NOT NULL,
    end_byte     INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    UNIQUE(file_path, para_index),
    FOREIGN KEY(file_path) REFERENCES files(path) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS chunks (
    id           INTEGER PRIMARY KEY,
    file_path    TEXT NOT NULL,
    chunk_index  INTEGER NOT NULL,
    total_chunks INTEGER NOT NULL,
    start_byte   INTEGER NOT NULL,
    end_byte     INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    preview      TEXT NOT NULL,
    UNIQUE(file_path, chunk_index)
);
CREATE INDEX IF NOT EXISTS idx_chunks_file     ON chunks(file_path);
CREATE INDEX IF NOT EXISTS idx_paragraphs_file ON paragraphs(file_path);
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
";

/// SQLite-backed store mapping chunk IDs <-> notes + paragraph hashes.
pub struct ChunkStore {
    #[cfg(feature = "local")]
    conn: Mutex<Connection>,
    #[cfg(not(feature = "local"))]
    _db_path: PathBuf,
}

impl ChunkStore {
    pub fn open(db_path: &Path) -> Result<Self, VectorError> {
        #[cfg(not(feature = "local"))]
        {
            let _ = db_path;
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let conn =
                Connection::open(db_path).map_err(|e| VectorError::Database(e.to_string()))?;
            conn.execute_batch(SCHEMA_SQL)
                .map_err(|e| VectorError::Database(e.to_string()))?;
            Ok(Self {
                conn: Mutex::new(conn),
            })
        }
    }

    pub fn get_chunks_for_file(&self, path: &str) -> Result<Vec<Chunk>, VectorError> {
        #[cfg(not(feature = "local"))]
        {
            let _ = path;
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, file_path, chunk_index, total_chunks, start_byte, end_byte, \
                     content_hash, preview FROM chunks WHERE file_path = ?1 ORDER BY chunk_index",
                )
                .map_err(|e| VectorError::Database(e.to_string()))?;

            let rows = stmt
                .query_map(params![path], |row| {
                    Ok(Chunk {
                        id: row.get::<_, i64>(0)? as u64,
                        note_path: row.get::<_, String>(1)?,
                        chunk_index: row.get::<_, i64>(2)? as u32,
                        total_chunks: row.get::<_, i64>(3)? as u32,
                        start_byte: row.get::<_, i64>(4)? as u64,
                        end_byte: row.get::<_, i64>(5)? as u64,
                        content_hash: row.get::<_, String>(6)?,
                        preview: row.get::<_, String>(7)?,
                    })
                })
                .map_err(|e| VectorError::Database(e.to_string()))?;

            let mut chunks = Vec::new();
            for row in rows {
                chunks.push(row.map_err(|e| VectorError::Database(e.to_string()))?);
            }
            Ok(chunks)
        }
    }

    pub fn insert_chunk(&self, chunk: &Chunk) -> Result<(), VectorError> {
        #[cfg(not(feature = "local"))]
        {
            let _ = chunk;
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO chunks \
                 (id, file_path, chunk_index, total_chunks, start_byte, end_byte, content_hash, preview) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    chunk.id as i64,
                    chunk.note_path,
                    chunk.chunk_index as i64,
                    chunk.total_chunks as i64,
                    chunk.start_byte as i64,
                    chunk.end_byte as i64,
                    chunk.content_hash,
                    chunk.preview,
                ],
            )
            .map_err(|e| VectorError::Database(e.to_string()))?;
            Ok(())
        }
    }

    pub fn delete_chunks_for_file(&self, path: &str) -> Result<Vec<u64>, VectorError> {
        #[cfg(not(feature = "local"))]
        {
            let _ = path;
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let conn = self.conn.lock().unwrap();
            // Collect IDs first.
            let mut stmt = conn
                .prepare("SELECT id FROM chunks WHERE file_path = ?1")
                .map_err(|e| VectorError::Database(e.to_string()))?;
            let ids: Vec<u64> = stmt
                .query_map(params![path], |row| row.get::<_, i64>(0))
                .map_err(|e| VectorError::Database(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| VectorError::Database(e.to_string()))?
                .into_iter()
                .map(|v: i64| v as u64)
                .collect();

            conn.execute("DELETE FROM chunks WHERE file_path = ?1", params![path])
                .map_err(|e| VectorError::Database(e.to_string()))?;

            Ok(ids)
        }
    }

    pub fn get_file_mtime(&self, path: &str) -> Result<Option<i64>, VectorError> {
        #[cfg(not(feature = "local"))]
        {
            let _ = path;
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT mtime FROM files WHERE path = ?1")
                .map_err(|e| VectorError::Database(e.to_string()))?;
            let mut rows = stmt
                .query_map(params![path], |row| row.get::<_, i64>(0))
                .map_err(|e| VectorError::Database(e.to_string()))?;
            match rows.next() {
                Some(r) => Ok(Some(r.map_err(|e| VectorError::Database(e.to_string()))?)),
                None => Ok(None),
            }
        }
    }

    pub fn upsert_file(&self, path: &str, mtime: i64, hash: &str) -> Result<(), VectorError> {
        #[cfg(not(feature = "local"))]
        {
            let _ = (path, mtime, hash);
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO files (path, mtime, content_hash) VALUES (?1, ?2, ?3)",
                params![path, mtime, hash],
            )
            .map_err(|e| VectorError::Database(e.to_string()))?;
            Ok(())
        }
    }

    pub fn all_indexed_paths(&self) -> Result<Vec<String>, VectorError> {
        #[cfg(not(feature = "local"))]
        {
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT path FROM files ORDER BY path")
                .map_err(|e| VectorError::Database(e.to_string()))?;
            let paths: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| VectorError::Database(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| VectorError::Database(e.to_string()))?;
            Ok(paths)
        }
    }

    pub fn get_model_meta(&self) -> Result<Option<(String, usize)>, VectorError> {
        #[cfg(not(feature = "local"))]
        {
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT key, value FROM meta WHERE key IN ('model', 'embedding_dims')")
                .map_err(|e| VectorError::Database(e.to_string()))?;
            let rows: Vec<(String, String)> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| VectorError::Database(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| VectorError::Database(e.to_string()))?;

            let mut model_opt: Option<String> = None;
            let mut dims_opt: Option<usize> = None;
            for (k, v) in rows {
                if k == "model" {
                    model_opt = Some(v);
                } else if k == "embedding_dims" {
                    dims_opt = v.parse::<usize>().ok();
                }
            }

            match (model_opt, dims_opt) {
                (Some(model), Some(dims)) => Ok(Some((model, dims))),
                _ => Ok(None),
            }
        }
    }

    pub fn set_model_meta(&self, model: &str, dims: usize) -> Result<(), VectorError> {
        #[cfg(not(feature = "local"))]
        {
            let _ = (model, dims);
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('model', ?1)",
                params![model],
            )
            .map_err(|e| VectorError::Database(e.to_string()))?;
            conn.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES ('embedding_dims', ?1)",
                params![dims.to_string()],
            )
            .map_err(|e| VectorError::Database(e.to_string()))?;
            Ok(())
        }
    }

    pub fn clear_all(&self) -> Result<(), VectorError> {
        #[cfg(not(feature = "local"))]
        {
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let conn = self.conn.lock().unwrap();
            conn.execute_batch(
                "DELETE FROM chunks; DELETE FROM paragraphs; DELETE FROM files; DELETE FROM meta;",
            )
            .map_err(|e| VectorError::Database(e.to_string()))?;
            Ok(())
        }
    }

    /// Lightweight query for diffing: returns `(id, content_hash)` pairs ordered by chunk_index,
    /// avoiding deserializing the full Chunk struct when only these two fields are needed.
    pub fn get_chunk_hashes_for_file(&self, path: &str) -> Result<Vec<(u64, String)>, VectorError> {
        #[cfg(not(feature = "local"))]
        {
            let _ = path;
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, content_hash FROM chunks WHERE file_path = ?1 ORDER BY chunk_index",
                )
                .map_err(|e| VectorError::Database(e.to_string()))?;

            let rows = stmt
                .query_map(params![path], |row| {
                    Ok((row.get::<_, i64>(0)? as u64, row.get::<_, String>(1)?))
                })
                .map_err(|e| VectorError::Database(e.to_string()))?;

            let mut result = Vec::new();
            for row in rows {
                result.push(row.map_err(|e| VectorError::Database(e.to_string()))?);
            }
            Ok(result)
        }
    }

    /// Insert all chunks for a file plus the file record in one transaction.
    ///
    /// Significantly faster than individual auto-commit inserts for multi-chunk files;
    /// SQLite auto-commit flushes the WAL on every statement, so N individual inserts
    /// cost O(N) fsyncs while one transaction costs O(1).
    pub fn insert_chunks_tx(
        &self,
        file_path: &str,
        mtime: i64,
        file_hash: &str,
        chunks: &[Chunk],
    ) -> Result<(), VectorError> {
        #[cfg(not(feature = "local"))]
        {
            let _ = (file_path, mtime, file_hash, chunks);
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let mut conn = self.conn.lock().unwrap();
            let tx = conn
                .transaction()
                .map_err(|e| VectorError::Database(e.to_string()))?;

            tx.execute(
                "INSERT OR REPLACE INTO files (path, mtime, content_hash) VALUES (?1, ?2, ?3)",
                params![file_path, mtime, file_hash],
            )
            .map_err(|e| VectorError::Database(e.to_string()))?;

            for chunk in chunks {
                tx.execute(
                    "INSERT OR REPLACE INTO chunks \
                     (id, file_path, chunk_index, total_chunks, start_byte, end_byte, content_hash, preview) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        chunk.id as i64,
                        chunk.note_path,
                        chunk.chunk_index as i64,
                        chunk.total_chunks as i64,
                        chunk.start_byte as i64,
                        chunk.end_byte as i64,
                        chunk.content_hash,
                        chunk.preview,
                    ],
                )
                .map_err(|e| VectorError::Database(e.to_string()))?;
            }

            tx.commit().map_err(|e| VectorError::Database(e.to_string()))?;
            Ok(())
        }
    }

    /// Delete stale chunk IDs, insert the new chunk set, and update the file record
    /// in one transaction. Uses a single `IN (...)` DELETE for the stale IDs.
    pub fn update_file_chunks_tx(
        &self,
        file_path: &str,
        mtime: i64,
        file_hash: &str,
        delete_ids: &[u64],
        chunks: &[Chunk],
    ) -> Result<(), VectorError> {
        #[cfg(not(feature = "local"))]
        {
            let _ = (file_path, mtime, file_hash, delete_ids, chunks);
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let mut conn = self.conn.lock().unwrap();
            let tx = conn
                .transaction()
                .map_err(|e| VectorError::Database(e.to_string()))?;

            if !delete_ids.is_empty() {
                let placeholders: String = std::iter::repeat_n("?", delete_ids.len())
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!("DELETE FROM chunks WHERE id IN ({placeholders})");
                let params: Vec<i64> = delete_ids.iter().map(|&id| id as i64).collect();
                tx.execute(&sql, rusqlite::params_from_iter(params.iter()))
                    .map_err(|e| VectorError::Database(e.to_string()))?;
            }

            for chunk in chunks {
                tx.execute(
                    "INSERT OR REPLACE INTO chunks \
                     (id, file_path, chunk_index, total_chunks, start_byte, end_byte, content_hash, preview) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        chunk.id as i64,
                        chunk.note_path,
                        chunk.chunk_index as i64,
                        chunk.total_chunks as i64,
                        chunk.start_byte as i64,
                        chunk.end_byte as i64,
                        chunk.content_hash,
                        chunk.preview,
                    ],
                )
                .map_err(|e| VectorError::Database(e.to_string()))?;
            }

            tx.execute(
                "INSERT OR REPLACE INTO files (path, mtime, content_hash) VALUES (?1, ?2, ?3)",
                params![file_path, mtime, file_hash],
            )
            .map_err(|e| VectorError::Database(e.to_string()))?;

            tx.commit().map_err(|e| VectorError::Database(e.to_string()))?;
            Ok(())
        }
    }

    /// Delete chunk rows by ID using a single `IN (...)` statement.
    pub fn delete_chunk_ids(&self, ids: &[u64]) -> Result<(), VectorError> {
        if ids.is_empty() {
            return Ok(());
        }

        #[cfg(not(feature = "local"))]
        {
            let _ = ids;
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let placeholders: String = std::iter::repeat_n("?", ids.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("DELETE FROM chunks WHERE id IN ({placeholders})");
            let params: Vec<i64> = ids.iter().map(|&id| id as i64).collect();
            let conn = self.conn.lock().unwrap();
            conn.execute(&sql, rusqlite::params_from_iter(params.iter()))
                .map_err(|e| VectorError::Database(e.to_string()))?;
            Ok(())
        }
    }

    pub fn next_chunk_id(&self) -> Result<u64, VectorError> {
        #[cfg(not(feature = "local"))]
        {
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let conn = self.conn.lock().unwrap();
            let max_id: Option<i64> = conn
                .query_row("SELECT MAX(id) FROM chunks", [], |row| row.get(0))
                .map_err(|e| VectorError::Database(e.to_string()))?;
            Ok(max_id.map(|v| (v + 1) as u64).unwrap_or(1))
        }
    }

    pub fn get_chunk_by_id(&self, id: u64) -> Result<Option<Chunk>, VectorError> {
        #[cfg(not(feature = "local"))]
        {
            let _ = id;
            return Err(VectorError::Database(
                "vector-search feature not compiled in".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, file_path, chunk_index, total_chunks, start_byte, end_byte, \
                     content_hash, preview FROM chunks WHERE id = ?1",
                )
                .map_err(|e| VectorError::Database(e.to_string()))?;

            let mut rows = stmt
                .query_map(params![id as i64], |row| {
                    Ok(Chunk {
                        id: row.get::<_, i64>(0)? as u64,
                        note_path: row.get::<_, String>(1)?,
                        chunk_index: row.get::<_, i64>(2)? as u32,
                        total_chunks: row.get::<_, i64>(3)? as u32,
                        start_byte: row.get::<_, i64>(4)? as u64,
                        end_byte: row.get::<_, i64>(5)? as u64,
                        content_hash: row.get::<_, String>(6)?,
                        preview: row.get::<_, String>(7)?,
                    })
                })
                .map_err(|e| VectorError::Database(e.to_string()))?;

            match rows.next() {
                Some(r) => Ok(Some(r.map_err(|e| VectorError::Database(e.to_string()))?)),
                None => Ok(None),
            }
        }
    }
}

#[cfg(test)]
#[cfg(feature = "local")]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn open_store() -> (ChunkStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = ChunkStore::open(&dir.path().join("test.db")).unwrap();
        (store, dir)
    }

    fn make_chunk(id: u64, path: &str, idx: u32, total: u32, hash: &str) -> Chunk {
        Chunk {
            id,
            note_path: path.to_string(),
            chunk_index: idx,
            total_chunks: total,
            start_byte: (idx * 10) as u64,
            end_byte: (idx * 10 + 10) as u64,
            content_hash: hash.to_string(),
            preview: format!("preview {idx}"),
        }
    }

    #[test]
    fn get_chunk_hashes_empty() {
        let (store, _dir) = open_store();
        let result = store.get_chunk_hashes_for_file("nonexistent.md").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn get_chunk_hashes_returns_id_and_hash_in_index_order() {
        let (store, _dir) = open_store();
        let chunks = vec![
            make_chunk(1, "a.md", 0, 3, "hash_a"),
            make_chunk(2, "a.md", 1, 3, "hash_b"),
            make_chunk(3, "a.md", 2, 3, "hash_c"),
        ];
        store.insert_chunks_tx("a.md", 1000, "filehash", &chunks).unwrap();

        let result = store.get_chunk_hashes_for_file("a.md").unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], (1, "hash_a".to_string()));
        assert_eq!(result[1], (2, "hash_b".to_string()));
        assert_eq!(result[2], (3, "hash_c".to_string()));
    }

    #[test]
    fn get_chunk_hashes_isolated_by_file() {
        let (store, _dir) = open_store();
        store
            .insert_chunks_tx("a.md", 1, "h1", &[make_chunk(1, "a.md", 0, 1, "ha")])
            .unwrap();
        store
            .insert_chunks_tx("b.md", 2, "h2", &[make_chunk(2, "b.md", 0, 1, "hb")])
            .unwrap();

        let result = store.get_chunk_hashes_for_file("a.md").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, "ha");
    }

    #[test]
    fn insert_chunks_tx_commits_all_or_nothing() {
        let (store, _dir) = open_store();
        let chunks = vec![
            make_chunk(10, "f.md", 0, 2, "h0"),
            make_chunk(11, "f.md", 1, 2, "h1"),
        ];
        store.insert_chunks_tx("f.md", 999, "fhash", &chunks).unwrap();

        // Both chunks and the file record should be present.
        let hashes = store.get_chunk_hashes_for_file("f.md").unwrap();
        assert_eq!(hashes.len(), 2);
        assert_eq!(store.get_file_mtime("f.md").unwrap(), Some(999));
    }

    #[test]
    fn update_file_chunks_tx_deletes_old_and_inserts_new() {
        let (store, _dir) = open_store();

        // Initial state: 3 chunks
        store
            .insert_chunks_tx(
                "note.md",
                100,
                "oldhash",
                &[
                    make_chunk(1, "note.md", 0, 3, "a"),
                    make_chunk(2, "note.md", 1, 3, "b"),
                    make_chunk(3, "note.md", 2, 3, "c"),
                ],
            )
            .unwrap();

        // Update: delete chunk 2 (stale), keep 1 and 3, add new chunk 4 at idx 1.
        let new_chunks = vec![
            make_chunk(1, "note.md", 0, 3, "a"),
            make_chunk(4, "note.md", 1, 3, "new"),
            make_chunk(3, "note.md", 2, 3, "c"),
        ];
        store
            .update_file_chunks_tx("note.md", 200, "newhash", &[2], &new_chunks)
            .unwrap();

        let hashes = store.get_chunk_hashes_for_file("note.md").unwrap();
        assert_eq!(hashes.len(), 3);
        // id=2 was deleted; id=4 is new
        let ids: Vec<u64> = hashes.iter().map(|(id, _)| *id).collect();
        assert!(!ids.contains(&2));
        assert!(ids.contains(&4));
        assert_eq!(store.get_file_mtime("note.md").unwrap(), Some(200));
    }

    #[test]
    fn delete_chunk_ids_in_clause() {
        let (store, _dir) = open_store();
        store
            .insert_chunks_tx(
                "x.md",
                1,
                "xh",
                &[
                    make_chunk(5, "x.md", 0, 3, "h5"),
                    make_chunk(6, "x.md", 1, 3, "h6"),
                    make_chunk(7, "x.md", 2, 3, "h7"),
                ],
            )
            .unwrap();

        store.delete_chunk_ids(&[5, 7]).unwrap();

        let hashes = store.get_chunk_hashes_for_file("x.md").unwrap();
        assert_eq!(hashes.len(), 1);
        assert_eq!(hashes[0].0, 6);
    }

    #[test]
    fn delete_chunk_ids_empty_is_noop() {
        let (store, _dir) = open_store();
        store.delete_chunk_ids(&[]).unwrap();
    }

    #[test]
    fn next_chunk_id_increments_after_insert() {
        let (store, _dir) = open_store();
        assert_eq!(store.next_chunk_id().unwrap(), 1);

        store
            .insert_chunks_tx("a.md", 1, "h", &[make_chunk(1, "a.md", 0, 1, "ha")])
            .unwrap();
        assert_eq!(store.next_chunk_id().unwrap(), 2);

        store
            .insert_chunks_tx("b.md", 2, "h2", &[make_chunk(10, "b.md", 0, 1, "hb")])
            .unwrap();
        assert_eq!(store.next_chunk_id().unwrap(), 11);
    }
}

/// Split `text` into `(start_byte, end_byte)` chunks with optional overlap.
///
/// Algorithm:
/// 1. Split on `\n\n` to get paragraphs.
/// 2. If a paragraph fits in `max_chars`, it becomes one segment.
/// 3. If too long, split on `. ` (sentence boundary).
/// 4. If a sentence is still too long, hard-split at `max_chars` char boundary.
/// 5. Apply overlap: prepend up to `overlap_chars` bytes from the end of the
///    previous segment to each subsequent segment's start.
pub fn chunk_text(text: &str, max_chars: usize, overlap_chars: usize) -> Vec<(usize, usize)> {
    if text.is_empty() || max_chars == 0 {
        return vec![];
    }

    // Build a list of (start_byte, end_byte) segments without overlap first.
    let mut segments: Vec<(usize, usize)> = Vec::new();

    // Split on paragraph boundaries (\n\n).
    // We walk the text manually to keep byte offsets accurate.
    let mut para_start: usize = 0;
    let mut remaining = text;

    loop {
        match remaining.find("\n\n") {
            Some(pos) => {
                let para = &remaining[..pos];
                if !para.is_empty() {
                    split_paragraph(para, para_start, max_chars, &mut segments);
                }
                // Skip the "\n\n" separator.
                para_start += pos + 2;
                remaining = &remaining[pos + 2..];
            }
            None => {
                // Last paragraph.
                if !remaining.is_empty() {
                    split_paragraph(remaining, para_start, max_chars, &mut segments);
                }
                break;
            }
        }
    }

    if segments.is_empty() {
        return segments;
    }

    // Apply overlap: adjust start_byte of each segment back by up to overlap_chars.
    if overlap_chars == 0 {
        return segments;
    }

    let mut result: Vec<(usize, usize)> = Vec::with_capacity(segments.len());
    result.push(segments[0]);

    for i in 1..segments.len() {
        let (prev_start, prev_end) = segments[i - 1];
        let (cur_start, cur_end) = segments[i];
        let prev_len = prev_end.saturating_sub(prev_start);
        let back = overlap_chars.min(prev_len);
        let new_start = cur_start.saturating_sub(back);
        result.push((new_start, cur_end));
    }

    result
}

/// Split a single paragraph into segments of at most `max_chars` chars,
/// appending byte-range pairs (relative to `text` start) into `segments`.
fn split_paragraph(para: &str, base: usize, max_chars: usize, segments: &mut Vec<(usize, usize)>) {
    let para_chars = para.chars().count();

    if para_chars <= max_chars {
        // Whole paragraph is one chunk.
        segments.push((base, base + para.len()));
        return;
    }

    // Too long — split on ". " sentence boundaries.
    let mut sent_start_byte: usize = 0;
    let mut sent_remaining = para;

    loop {
        match sent_remaining.find(". ") {
            Some(pos) => {
                // Sentence ends after the period (inclusive).
                let sentence = &sent_remaining[..pos + 1];
                let abs_start = base + sent_start_byte;
                push_hard_chunks(sentence, abs_start, max_chars, segments);

                // Advance past ". " (the space after the period).
                sent_start_byte += pos + 2;
                sent_remaining = &sent_remaining[pos + 2..];
            }
            None => {
                // Last sentence fragment.
                if !sent_remaining.is_empty() {
                    let abs_start = base + sent_start_byte;
                    push_hard_chunks(sent_remaining, abs_start, max_chars, segments);
                }
                break;
            }
        }
    }
}

/// Push one or more hard-split chunks from `text` (which has absolute start
/// byte `base`) each of at most `max_chars` characters.
fn push_hard_chunks(text: &str, base: usize, max_chars: usize, segments: &mut Vec<(usize, usize)>) {
    let mut char_count = 0usize;
    let mut chunk_start_byte: usize = 0; // relative to `text`

    for (byte_idx, _ch) in text.char_indices() {
        if char_count == max_chars {
            // Emit the current chunk.
            segments.push((base + chunk_start_byte, base + byte_idx));
            chunk_start_byte = byte_idx;
            char_count = 0;
        }
        char_count += 1;
    }

    // Emit the final (possibly partial) chunk.
    if chunk_start_byte < text.len() {
        segments.push((base + chunk_start_byte, base + text.len()));
    }
}
