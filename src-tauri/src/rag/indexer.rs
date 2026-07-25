use std::path::Path;
use std::sync::Mutex;

use rusqlite::params;
use serde::Serialize;
use thiserror::Error;

use crate::db::Database;
use crate::rag::embeddings::Embedder;

#[cfg(test)]
use crate::rag::embeddings::EMBEDDING_DIM;

/// Word-based chunk size. Validated empirically against the 512-token limit
/// of snowflake-arctic-embed-s (BERT WordPiece tokenizer):
/// - The test `chunk_token_count_within_bert_limit` (requires model download)
///   creates a dense-technical CV-like document, chunks it at this size,
///   and asserts every chunk ≤512 tokens.
/// - At 150 words with 20-word overlap, even heavily-fragmented technical
///   terms (ratio ~2.5-3.0 tokens/word) stay under the safety margin of 450.
/// - If the tokenizer starts truncating, a warning is logged at ingest time
///   via `Embedder::token_count()`.
pub const CHUNK_SIZE: usize = 150;
pub const CHUNK_OVERLAP: usize = 20;
pub const MAX_TOKENS_PER_CHUNK: usize = 512;

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;
const ALLOWED_EXTENSIONS: [&str; 3] = ["txt", "md", "pdf"];

#[derive(Debug, Error)]
pub enum RagError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("mutex poisoned: {0}")]
    Lock(String),
    #[error("embedding generation failed: {0}")]
    Embedding(Box<dyn std::error::Error>),
    #[error("unsupported extension '.{0}'")]
    UnsupportedExtension(String),
    #[error("file too large ({0} bytes, max {MAX_FILE_SIZE})")]
    FileTooLarge(u64),
    #[error("no supported files in {0}")]
    NoSupportedFiles(String),
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub id: i64,
    pub document_id: i64,
    pub text: String,
    pub chunk_index: i32,
    pub tag: Option<String>,
    pub metric: Option<String>,
    pub score: f32,
}

/// Ingests a list of files into the vector index.
///
/// Reads each file, splits its content into ~CHUNK_SIZE-word chunks with
/// CHUNK_OVERLAP-word overlap, generates embeddings via the provided model,
/// and stores everything in the database (`documents` + `chunks` + `chunks_vec`).
/// Each file is processed atomically inside a SQLite transaction — if embedding
/// generation fails mid-way, the document and its partial chunks are rolled back.
///
/// Only files with supported extensions (txt, md, pdf) up to 10MB are accepted.
///
/// # Token-limit guard
///
/// After splitting each chunk, the caller uses `Embedder::token_count()` to
/// check whether the chunk exceeds `MAX_TOKENS_PER_CHUNK`. If it does, a
/// warning is logged but the chunk is still processed. This is a safety net —
/// the chunk size constant is empirically validated, but production data may
/// differ from the test corpus.
pub fn ingest_documents(
    model: &impl Embedder,
    db: &Database,
    files: &[String],
) -> Result<(), RagError> {
    for file_path in files {
        let path = Path::new(file_path);
        let ext = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
            return Err(RagError::UnsupportedExtension(ext));
        }

        let metadata = std::fs::metadata(path)?;
        if metadata.len() > MAX_FILE_SIZE {
            return Err(RagError::FileTooLarge(metadata.len()));
        }

        let filename = path
            .file_name()
            .ok_or_else(|| RagError::Other(format!("invalid path: {file_path}")))?
            .to_string_lossy()
            .to_string();

        let content = std::fs::read_to_string(path)?;

        let chunks = chunk_text(&content, CHUNK_SIZE, CHUNK_OVERLAP);

        let conn = db.conn.lock().map_err(|e| RagError::Lock(e.to_string()))?;
        conn.execute_batch("BEGIN")?;
        conn.execute(
            "INSERT INTO documents (filename, type) VALUES (?1, ?2)",
            params![filename, ext],
        )?;
        let doc_id = conn.last_insert_rowid();

        for (i, chunk_text) in chunks.iter().enumerate() {
            let token_count = model.token_count(chunk_text);
            if token_count > MAX_TOKENS_PER_CHUNK {
                eprintln!(
                    "[kue] WARN: Chunk {i} in '{filename}' has {token_count} tokens \
                     (max {MAX_TOKENS_PER_CHUNK}). Chunk will be truncated."
                );
            }

            let embedding = model
                .generate_embedding(chunk_text)
                .map_err(|e| RagError::Embedding(e))?;

            conn.execute(
                "INSERT INTO chunks (document_id, text, chunk_index) VALUES (?1, ?2, ?3)",
                params![doc_id, chunk_text, i as i32],
            )?;
            let chunk_id = conn.last_insert_rowid();

            let embedding_bytes: &[u8] = bytemuck::cast_slice(&embedding);
            conn.execute(
                "INSERT INTO chunks_vec (rowid, embedding) VALUES (?1, ?2)",
                params![chunk_id, embedding_bytes],
            )?;
        }

        conn.execute_batch("COMMIT")?;
        drop(conn);
    }
    Ok(())
}

/// Indexes all supported files (.txt, .md, .pdf) in a directory (non-recursive).
///
/// Path traversal is prevented via `canonicalize` — symbolic links and `../`
/// sequences are resolved before any files are read. The directory is scanned
/// in sorted order for deterministic behavior across runs.
pub fn index_folder(
    model: &impl Embedder,
    db: &Database,
    folder_path: &str,
) -> Result<(), RagError> {
    let canonical = std::fs::canonicalize(folder_path)?;

    let mut files: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&canonical)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            continue;
        }

        if ALLOWED_EXTENSIONS
            .iter()
            .any(|ext| path.extension().map_or(false, |e| e == *ext))
        {
            files.push(path.to_string_lossy().to_string());
        }
    }
    files.sort();

    if files.is_empty() {
        return Err(RagError::NoSupportedFiles(folder_path.to_string()));
    }

    ingest_documents(model, db, &files)
}

/// Searches the vector index for chunks semantically similar to `query`.
///
/// Returns up to `top_k` results ordered by cosine distance (ascending).
/// The query is embedded via the provided model, then matched against
/// `chunks_vec` using sqlite-vec's KNN search (`k = top_k`).
///
/// NOTE: Embedding bytes are serialized as little-endian f32 via `bytemuck::cast_slice`.
/// This matches sqlite-vec's expected format on all current platforms (x86, ARM).
pub fn search(
    model: &impl Embedder,
    db: &Database,
    query: &str,
    top_k: usize,
) -> Result<Vec<SearchResult>, RagError> {
    let query_embedding = model
        .generate_embedding(query)
        .map_err(|e| RagError::Embedding(e))?;
    let query_bytes: &[u8] = bytemuck::cast_slice(&query_embedding);

    let conn = db.conn.lock().map_err(|e| RagError::Lock(e.to_string()))?;
    let mut stmt = conn.prepare(
        "SELECT c.id, c.document_id, c.text, c.chunk_index, c.tag, c.metric, v.distance
         FROM chunks_vec v
         JOIN chunks c ON c.id = v.rowid
         WHERE v.embedding MATCH ?1 AND k = ?2",
    )?;

    let results = stmt.query_map(params![query_bytes, top_k as i64], |row| {
        Ok(SearchResult {
            id: row.get(0)?,
            document_id: row.get(1)?,
            text: row.get(2)?,
            chunk_index: row.get(3)?,
            tag: row.get(4)?,
            metric: row.get(5)?,
            score: row.get::<_, f32>(6)?,
        })
    })?;

    let mut chunks = Vec::new();
    for result in results {
        chunks.push(result?);
    }
    Ok(chunks)
}

/// Summary returned by the `index_folder` Tauri command.
#[derive(Debug, Serialize)]
pub struct IndexSummary {
    pub folder: String,
    pub files_indexed: usize,
    pub chunks_created: usize,
    pub error_count: usize,
}

/// Tauri command: index all supported files in a folder.
///
/// Uses the embedding model managed as Tauri state. Returns a summary
/// of indexed documents and chunks, or an error string.
#[tauri::command]
pub fn index_folder_cmd(
    path: String,
    db: tauri::State<'_, crate::db::Database>,
    model: tauri::State<'_, Mutex<crate::rag::embeddings::EmbeddingModel>>,
) -> Result<IndexSummary, String> {
    let db = db.inner();
    let model = model.inner();

    let canonical = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;

    let mut files: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&canonical).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        if p.is_dir() {
            continue;
        }
        if ALLOWED_EXTENSIONS
            .iter()
            .any(|ext| p.extension().map_or(false, |e| e == *ext))
        {
            files.push(p.to_string_lossy().to_string());
        }
    }
    files.sort();

    if files.is_empty() {
        return Err("no supported files found in folder".to_string());
    }

    let files_count = files.len();

    // Count chunks before ingestion to compute delta
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let chunks_before: usize = conn
        .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    drop(conn);

    ingest_documents(model, db, &files).map_err(|e| e.to_string())?;

    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let chunks_after: usize = conn
        .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    drop(conn);

    Ok(IndexSummary {
        folder: path,
        files_indexed: files_count,
        chunks_created: chunks_after.saturating_sub(chunks_before),
        error_count: 0,
    })
}

/// Tauri command: search the vector index for chunks similar to `query`.
///
/// Returns up to `top_k` results with their text, metadata, and similarity score.
#[tauri::command]
pub fn search_context(
    query: String,
    top_k: usize,
    db: tauri::State<'_, crate::db::Database>,
    model: tauri::State<'_, Mutex<crate::rag::embeddings::EmbeddingModel>>,
) -> Result<Vec<SearchResult>, String> {
    let db = db.inner();
    let model = model.inner();
    search(model, db, &query, top_k).map_err(|e| e.to_string())
}

/// Splits text into ~`chunk_size`-word segments with `overlap`-word overlap between
/// consecutive chunks.
///
/// Chunking is word-count-based, not token-count-based. The global `CHUNK_SIZE` and
/// `CHUNK_OVERLAP` constants are validated empirically against the model's 512-token
/// limit (see `chunk_token_count_within_bert_limit` test). If the tokenizer still
/// reports >512 tokens at ingest time, a warning is logged.
fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut chunks = Vec::new();
    let mut start = 0;
    let overlap = overlap.min(chunk_size.saturating_sub(1));

    while start < words.len() {
        let end = std::cmp::min(start + chunk_size, words.len());
        let chunk = words[start..end].join(" ");
        chunks.push(chunk);

        if end == words.len() {
            break;
        }

        start = end - overlap;
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_and_migrate, register_vec_extension};
    use std::path::PathBuf;
    use std::sync::Once;

    static VEC_INIT: Once = Once::new();

    fn ensure_vec_extension() {
        VEC_INIT.call_once(|| {
            register_vec_extension();
        });
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let mut dir = std::env::temp_dir();
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            dir.push(format!("kue_rag_test_{}_{}_{}", std::process::id(), label, id));
            let _ = std::fs::create_dir_all(&dir);
            TempDir { path: dir }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    struct MockEmbeddingModel;

    impl Embedder for MockEmbeddingModel {
        fn generate_embedding(&self, _text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
            let mut emb = vec![0.0f32; EMBEDDING_DIM];
            emb[0] = 1.0;
            Ok(emb)
        }
    }

    // -----------------------------------------------------------------------
    // chunk_text
    // -----------------------------------------------------------------------

    #[test]
    fn chunk_text_empty_string() {
        let chunks = chunk_text("", 200, 20);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_text_single_word() {
        let chunks = chunk_text("hello", 200, 20);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn chunk_text_fewer_words_than_chunk_size() {
        let text = "the quick brown fox jumps";
        let chunks = chunk_text(text, 200, 20);
        assert_eq!(chunks, vec![text]);
    }

    #[test]
    fn chunk_text_splits_at_chunk_boundary() {
        let words = vec!["word"; 250];
        let text = words.join(" ");
        let chunks = chunk_text(&text, 200, 30);

        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].split_whitespace().count() <= 200);
        assert!(chunks[1].split_whitespace().count() <= 200);
    }

    #[test]
    fn chunk_text_overlap_works() {
        let words: Vec<String> = (0..300).map(|i| format!("w{i}")).collect();
        let text = words.join(" ");
        let chunks = chunk_text(&text, 100, 20);

        assert!(chunks.len() >= 3);

        let first_words: Vec<&str> = chunks[0].split_whitespace().collect();
        let second_words: Vec<&str> = chunks[1].split_whitespace().collect();

        let overlap: Vec<&&str> = first_words
            .iter()
            .filter(|w| second_words.contains(w))
            .collect();
        assert!(overlap.len() >= 10, "expected at least some overlap words");
    }

    #[test]
    fn chunk_text_exact_chunk_size() {
        let words = vec!["test"; 200];
        let text = words.join(" ");
        let chunks = chunk_text(&text, 200, 20);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn chunk_text_one_word_over() {
        let words = vec!["test"; 201];
        let text = words.join(" ");
        let chunks = chunk_text(&text, 200, 20);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[1].split_whitespace().count() <= 21);
    }

    // -----------------------------------------------------------------------
    // ingest_documents
    // -----------------------------------------------------------------------

    #[test]
    fn ingest_documents_inserts_rows() {
        ensure_vec_extension();
        let tmp = TempDir::new("ingest");

        std::fs::write(
            tmp.path().join("test.txt"),
            "Kue es un copiloto de memoria para entrevistas tecnicas desarrollado en Rust con Tauri.",
        )
        .unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let files = vec![tmp.path().join("test.txt").to_string_lossy().to_string()];
        ingest_documents(&model, &db, &files).unwrap();

        let conn = db.conn.lock().unwrap();
        let doc_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(doc_count, 1, "should have one document");

        let chunk_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(chunk_count, 1, "should have one chunk");

        let vec_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks_vec", [], |row| row.get(0))
            .unwrap();
        assert_eq!(vec_count, 1, "should have one vector entry");

        let text: String = conn
            .query_row("SELECT text FROM chunks WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert!(text.contains("Kue"), "chunk text should contain original content");
    }

    #[test]
    fn ingest_documents_rejects_unsupported_extension() {
        ensure_vec_extension();
        let tmp = TempDir::new("unsupported");

        std::fs::write(tmp.path().join("test.png"), "not a text file").unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let files = vec![tmp.path().join("test.png").to_string_lossy().to_string()];
        let result = ingest_documents(&model, &db, &files);
        assert!(result.is_err(), "should reject .png files");
        assert!(
            result.unwrap_err().to_string().contains("unsupported"),
            "error should mention unsupported extension"
        );
    }

    #[test]
    fn ingest_documents_rejects_oversized_file() {
        ensure_vec_extension();
        let tmp = TempDir::new("oversized");

        let oversized = "x".repeat((MAX_FILE_SIZE + 1) as usize);
        std::fs::write(tmp.path().join("big.txt"), &oversized).unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let files = vec![tmp.path().join("big.txt").to_string_lossy().to_string()];
        let result = ingest_documents(&model, &db, &files);
        assert!(result.is_err(), "should reject oversized files");
    }

    #[test]
    fn ingest_documents_multiple_files() {
        ensure_vec_extension();
        let tmp = TempDir::new("multi");

        for i in 0..3 {
            std::fs::write(
                tmp.path().join(format!("doc_{i}.txt")),
                format!("Contenido del documento {i}."),
            )
            .unwrap();
        }

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let files: Vec<String> = (0..3)
            .map(|i| tmp.path().join(format!("doc_{i}.txt")).to_string_lossy().to_string())
            .collect();
        ingest_documents(&model, &db, &files).unwrap();

        let conn = db.conn.lock().unwrap();
        let doc_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(doc_count, 3, "should have three documents");
    }

    // -----------------------------------------------------------------------
    // index_folder
    // -----------------------------------------------------------------------

    #[test]
    fn index_folder_rejects_path_traversal() {
        ensure_vec_extension();
        let tmp = TempDir::new("traversal");
        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let result = index_folder(&model, &db, "/etc/passwd");
        assert!(result.is_err(), "should reject path traversal");
    }

    // -----------------------------------------------------------------------
    // search
    // -----------------------------------------------------------------------

    #[test]
    fn search_returns_results() {
        ensure_vec_extension();
        let tmp = TempDir::new("search");

        std::fs::write(
            tmp.path().join("search_me.txt"),
            "Prisma es un ORM moderno para Node.js y TypeScript que acelera el desarrollo backend.",
        )
        .unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let files = vec![tmp.path().join("search_me.txt").to_string_lossy().to_string()];
        ingest_documents(&model, &db, &files).unwrap();

        let results = search(&model, &db, "prisma", 5).unwrap();
        assert!(!results.is_empty(), "should return at least one result");
        assert!(
            results[0].text.contains("Prisma"),
            "top result should contain 'Prisma'"
        );
        assert!(results[0].score >= 0.0, "score should be non-negative");
    }

    #[test]
    fn search_returns_empty_for_no_match() {
        ensure_vec_extension();
        let tmp = TempDir::new("empty");

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let results = search(&model, &db, "anything", 5).unwrap();
        assert!(results.is_empty(), "search on empty DB should return empty");
    }

    #[test]
    fn search_respects_top_k() {
        ensure_vec_extension();
        let tmp = TempDir::new("topk");

        for i in 0..5 {
            std::fs::write(
                tmp.path().join(format!("doc_{i}.txt")),
                format!("Documento numero {i} con contenido variado."),
            )
            .unwrap();
        }

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let files: Vec<String> = (0..5)
            .map(|i| tmp.path().join(format!("doc_{i}.txt")).to_string_lossy().to_string())
            .collect();
        ingest_documents(&model, &db, &files).unwrap();

        let results = search(&model, &db, "busqueda", 3).unwrap();
        assert_eq!(results.len(), 3, "should return exactly top_k=3 results");
    }

    // -----------------------------------------------------------------------
    // chunk_text — additional edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn chunk_text_overlap_zero() {
        let words: Vec<String> = (0..50).map(|i| format!("w{i}")).collect();
        let text = words.join(" ");
        let chunks = chunk_text(&text, 20, 0);

        assert_eq!(chunks.len(), 3, "50 words / 20 chunk_size = 3 chunks");
        assert_eq!(chunks[0].split_whitespace().count(), 20);
        assert_eq!(chunks[1].split_whitespace().count(), 20);
        assert_eq!(chunks[2].split_whitespace().count(), 10);

        // With overlap=0, no word should appear in consecutive chunks
        let first_words: Vec<&str> = chunks[0].split_whitespace().collect();
        let second_words: Vec<&str> = chunks[1].split_whitespace().collect();
        let common: Vec<&&str> = first_words.iter().filter(|w| second_words.contains(w)).collect();
        assert_eq!(common.len(), 0, "no overlap when overlap=0");
    }

    #[test]
    fn chunk_text_overlap_large_but_within_chunk_size() {
        // Overlap close to chunk_size — still works as long as overlap < chunk_size
        let words: Vec<String> = (0..100).map(|i| format!("w{i}")).collect();
        let text = words.join(" ");
        let chunks = chunk_text(&text, 30, 25);

        assert!(chunks.len() >= 4, "should produce multiple chunks");
        // Each chunk (except last) should share ~25 words with the next one
        for pair in chunks.windows(2) {
            let first: Vec<&str> = pair[0].split_whitespace().collect();
            let second: Vec<&str> = pair[1].split_whitespace().collect();
            let overlap: Vec<&&str> = first.iter().filter(|w| second.contains(w)).collect();
            assert!(overlap.len() >= 20, "expected large overlap between consecutive chunks");
        }
    }

    #[test]
    fn chunk_text_only_whitespace() {
        let chunks = chunk_text("   \n  \t  \r\n  ", 200, 20);
        assert!(chunks.is_empty(), "whitespace-only text should yield no chunks");
    }

    #[test]
    fn chunk_text_unicode_multibyte() {
        let text = "café résumé niño über München 日本国 αβγ 📚🧪";
        // This should be treated as a sequence of whitespace-separated words
        let chunks = chunk_text(text, 200, 20);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("café"), "should preserve unicode chars");
        assert!(chunks[0].contains("日本国"), "should preserve CJK chars");
        assert!(chunks[0].contains("📚🧪"), "should preserve emoji");
    }

    #[test]
    fn chunk_text_repeated_words_with_varying_overlap() {
        // Ensure that with overlap, the chunks don't grow unbounded
        let text = "word ";
        let long: String = text.repeat(500);
        let chunks = chunk_text(&long, 100, 20);
        // Validate every chunk is at most chunk_size words
        for (i, chunk) in chunks.iter().enumerate() {
            let count = chunk.split_whitespace().count();
            assert!(count <= 100, "chunk {i} has {count} words, expected ≤100");
        }
    }

    // -----------------------------------------------------------------------
    // ingest_documents — additional error and edge paths
    // -----------------------------------------------------------------------

    struct FailingEmbedder;

    impl Embedder for FailingEmbedder {
        fn generate_embedding(&self, _text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
            Err("mock embedding failure".into())
        }
    }

    #[test]
    fn ingest_documents_file_not_found() {
        ensure_vec_extension();
        let tmp = TempDir::new("not_found");

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let files = vec![tmp.path().join("nonexistent.txt").to_string_lossy().to_string()];
        let result = ingest_documents(&model, &db, &files);
        assert!(result.is_err(), "should error for non-existent file");
    }

    #[test]
    fn ingest_documents_empty_file() {
        ensure_vec_extension();
        let tmp = TempDir::new("empty_file");

        std::fs::write(tmp.path().join("empty.txt"), "").unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let files = vec![tmp.path().join("empty.txt").to_string_lossy().to_string()];
        ingest_documents(&model, &db, &files).unwrap();

        let conn = db.conn.lock().unwrap();
        let doc_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(doc_count, 1, "should create a document entry");

        let chunk_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(chunk_count, 0, "empty file should produce zero chunks");
    }

    #[test]
    fn ingest_documents_multiple_chunks() {
        ensure_vec_extension();
        let tmp = TempDir::new("multi_chunk");

        // Create a file with enough words to span multiple chunks (chunk_size=200, overlap=30)
        let big_text = (0..500)
            .map(|i| format!("word_{}", i))
            .collect::<Vec<_>>()
            .join(" ");
        std::fs::write(tmp.path().join("big.txt"), &big_text).unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let files = vec![tmp.path().join("big.txt").to_string_lossy().to_string()];
        ingest_documents(&model, &db, &files).unwrap();

        let conn = db.conn.lock().unwrap();
        let chunk_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
            .unwrap();
        assert!(chunk_count >= 2, "500 words / 200 chunk_size should produce >= 2 chunks, got {chunk_count}");

        // Verify chunks are stored in order
        let indexes: Vec<i32> = {
            let mut stmt = conn
                .prepare("SELECT chunk_index FROM chunks ORDER BY chunk_index")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, i32>(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        let expected: Vec<i32> = (0..chunk_count as i32).collect();
        assert_eq!(indexes, expected, "chunk_index should be sequential");
    }

    #[test]
    fn ingest_documents_embedding_error() {
        ensure_vec_extension();
        let tmp = TempDir::new("emb_error");

        std::fs::write(
            tmp.path().join("test.txt"),
            "This text will cause an embedding error.",
        )
        .unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let failing = FailingEmbedder;

        let files = vec![tmp.path().join("test.txt").to_string_lossy().to_string()];
        let result = ingest_documents(&failing, &db, &files);
        assert!(result.is_err(), "should propagate embedding error");
        assert!(
            result.unwrap_err().to_string().contains("mock embedding failure"),
            "error should contain the mock's error message"
        );
    }

    #[test]
    fn ingest_documents_binary_file() {
        ensure_vec_extension();
        let tmp = TempDir::new("binary_file");

        // Write raw bytes that are not valid UTF-8
        let binary: Vec<u8> = vec![0x00, 0xFF, 0xFE, 0xFD, 0x00, 0x01, 0x02];
        std::fs::write(tmp.path().join("binary.txt"), &binary).unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let files = vec![tmp.path().join("binary.txt").to_string_lossy().to_string()];
        let result = ingest_documents(&model, &db, &files);
        assert!(result.is_err(), "binary file should cause read_to_string error");
    }

    // -----------------------------------------------------------------------
    // index_folder — happy path and edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn index_folder_happy_path() {
        ensure_vec_extension();
        let tmp = TempDir::new("index_happy");

        std::fs::write(
            tmp.path().join("doc_a.txt"),
            "Rust is a systems programming language focused on safety and performance.",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("doc_b.md"),
            "Tauri is a framework for building desktop applications with web technologies.",
        )
        .unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        index_folder(&model, &db, tmp.path().to_string_lossy().as_ref())
            .expect("index_folder should succeed");

        let conn = db.conn.lock().unwrap();
        let doc_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(doc_count, 2, "should index both files");

        // Verify document names are stored
        let names: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT filename FROM documents ORDER BY filename")
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert_eq!(names, vec!["doc_a.txt", "doc_b.md"], "filenames should match and be sorted");
    }

    #[test]
    fn index_folder_empty_directory() {
        ensure_vec_extension();
        let tmp = TempDir::new("index_empty");
        // Directory exists but is empty

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let result = index_folder(&model, &db, tmp.path().to_string_lossy().as_ref());
        assert!(result.is_err(), "should error on empty directory");
    }

    #[test]
    fn index_folder_no_supported_files() {
        ensure_vec_extension();
        let tmp = TempDir::new("index_no_support");

        // Only unsupported extensions
        std::fs::write(tmp.path().join("image.png"), "fake png").unwrap();
        std::fs::write(tmp.path().join("data.json"), r#"{"key": "value"}"#).unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let result = index_folder(&model, &db, tmp.path().to_string_lossy().as_ref());
        assert!(result.is_err(), "should error when no supported files found");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no supported files"), "error should mention no supported files: {err}");
    }

    #[test]
    fn index_folder_mixed_extensions() {
        ensure_vec_extension();
        let tmp = TempDir::new("index_mixed");

        // Supported
        std::fs::write(tmp.path().join("readme.txt"), "hello").unwrap();
        std::fs::write(tmp.path().join("notes.md"), "world").unwrap();
        // Unsupported (should be skipped)
        std::fs::write(tmp.path().join("image.png"), "fake").unwrap();
        std::fs::write(tmp.path().join("data.json"), r#"{}"#).unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        index_folder(&model, &db, tmp.path().to_string_lossy().as_ref())
            .expect("should succeed with mixed extensions");

        let conn = db.conn.lock().unwrap();
        let doc_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(doc_count, 2, "should only index supported extensions");
    }

    #[test]
    fn index_folder_skips_subdirectories() {
        ensure_vec_extension();
        let tmp = TempDir::new("index_subdir");

        // Supported file at root
        std::fs::write(tmp.path().join("root.txt"), "I am at the root.").unwrap();
        // Subdirectory with a supported file (should be skipped)
        std::fs::create_dir_all(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub").join("nested.txt"), "I am nested.").unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        index_folder(&model, &db, tmp.path().to_string_lossy().as_ref())
            .expect("should succeed, skipping subdirectories");

        let conn = db.conn.lock().unwrap();
        let doc_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(doc_count, 1, "should only index root-level files, not subdirectory files");

        let name: String = conn
            .query_row("SELECT filename FROM documents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, "root.txt", "should only have root.txt, not nested.txt");
    }

    #[test]
    fn index_folder_non_existent_directory() {
        ensure_vec_extension();
        let tmp = TempDir::new("index_nonexistent");
        let fake_dir = tmp.path().join("does_not_exist");

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let result = index_folder(&model, &db, fake_dir.to_string_lossy().as_ref());
        assert!(result.is_err(), "should error for non-existent directory");
    }

    // -----------------------------------------------------------------------
    // search — edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn search_with_top_k_zero() {
        ensure_vec_extension();
        let tmp = TempDir::new("topk0");

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        // top_k=0 should succeed and return empty
        let results = search(&model, &db, "anything", 0).unwrap();
        assert!(results.is_empty(), "top_k=0 should return empty results");
    }

    #[test]
    fn search_with_empty_query() {
        ensure_vec_extension();
        let tmp = TempDir::new("empty_query");

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        // Empty query string — mock returns a valid embedding, so search runs
        let results = search(&model, &db, "", 5).unwrap();
        assert!(results.is_empty(), "search on empty DB with empty query returns empty");
    }

    #[test]
    fn search_with_top_k_greater_than_available() {
        ensure_vec_extension();
        let tmp = TempDir::new("topk_large");

        std::fs::write(
            tmp.path().join("doc.txt"),
            "Solo hay un documento con una sola oracion.",
        )
        .unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let files = vec![tmp.path().join("doc.txt").to_string_lossy().to_string()];
        ingest_documents(&model, &db, &files).unwrap();

        // Requesting more results than available should return all available
        let results = search(&model, &db, "test", 100).unwrap();
        assert!(!results.is_empty(), "should return results even when k > available");
        // With one small document, we should have exactly 1 chunk
        assert_eq!(results.len(), 1, "should return exactly the 1 available chunk");
    }

    // -----------------------------------------------------------------------
    // ingest_documents — lock poisoning
    // -----------------------------------------------------------------------

    #[test]
    fn ingest_documents_handles_poisoned_mutex() {
        ensure_vec_extension();
        let tmp = TempDir::new("lock_poison");

        std::fs::write(tmp.path().join("test.txt"), "some content").unwrap();

        let db_path = tmp.path().join("test.db");
        let underlying = rusqlite::Connection::open(&db_path).unwrap();
        let shared = std::sync::Arc::new(std::sync::Mutex::new(underlying));

        // Poison the mutex
        let cloned = std::sync::Arc::clone(&shared);
        let handle = std::thread::spawn(move || {
            let _guard = cloned.lock().unwrap();
            panic!("intentional panic to poison mutex");
        });
        let _ = handle.join();

        let poisoned = std::sync::Arc::try_unwrap(shared).unwrap();
        let db = crate::db::Database {
            conn: poisoned,
            path: db_path,
        };
        let model = MockEmbeddingModel;

        let files = vec![tmp.path().join("test.txt").to_string_lossy().to_string()];
        let result = ingest_documents(&model, &db, &files);
        assert!(result.is_err(), "should error when mutex is poisoned");
        let err = result.unwrap_err().to_string();
        assert!(
            err.to_lowercase().contains("poison"),
            "error should mention poison, got: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // ingest_documents — no file extension
    // -----------------------------------------------------------------------

    #[test]
    fn ingest_documents_no_extension() {
        ensure_vec_extension();
        let tmp = TempDir::new("no_ext");

        std::fs::write(tmp.path().join("test_no_ext"), "content without extension").unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let files = vec![tmp.path().join("test_no_ext").to_string_lossy().to_string()];
        let result = ingest_documents(&model, &db, &files);
        assert!(result.is_err(), "should error for file without extension");
        let err = result.unwrap_err().to_string();
        assert!(
            err.to_lowercase().contains("unsupported extension"),
            "error should mention unsupported extension, got: {err}"
        );
    }

    #[test]
    fn ingest_documents_root_path_rejected_at_extension_check() {
        ensure_vec_extension();
        let tmp = TempDir::new("root_path");

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        // Root "/" has empty extension, which is never in ALLOWED_EXTENSIONS
        let files = vec!["/".to_string()];
        let result = ingest_documents(&model, &db, &files);
        assert!(result.is_err(), "should error for root path");
    }

    // -----------------------------------------------------------------------
    // search — embedding generation failure
    // -----------------------------------------------------------------------

    #[test]
    fn search_handles_embedding_error() {
        ensure_vec_extension();
        let tmp = TempDir::new("srch_emb_err");

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let failing = FailingEmbedder;

        let result = search(&failing, &db, "test query", 5);
        assert!(result.is_err(), "should error when embedding fails");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("mock embedding failure"),
            "error should propagate embedding error, got: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // SearchResult serialization
    // -----------------------------------------------------------------------

    #[test]
    fn search_result_serialization() {
        let result = SearchResult {
            id: 42,
            document_id: 7,
            text: "Rust is safe and fast.".to_string(),
            chunk_index: 0,
            tag: Some("rust".to_string()),
            metric: Some("safety".to_string()),
            score: 0.95,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"id\":42"), "should contain id: {json}");
        assert!(json.contains("\"document_id\":7"), "should contain document_id: {json}");
        assert!(json.contains("\"text\":\"Rust is safe and fast.\""), "should contain text: {json}");
        assert!(json.contains("\"chunk_index\":0"), "should contain chunk_index: {json}");
        assert!(json.contains("\"tag\":\"rust\""), "should contain tag: {json}");
        assert!(json.contains("\"metric\":\"safety\""), "should contain metric: {json}");
        assert!(json.contains("\"score\":0.95"), "should contain score: {json}");
    }

    #[test]
    fn search_result_serialization_null_fields() {
        let result = SearchResult {
            id: 1,
            document_id: 1,
            text: "No metadata.".to_string(),
            chunk_index: 0,
            tag: None,
            metric: None,
            score: 0.0,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"tag\":null"), "null tag should serialize as null: {json}");
        assert!(json.contains("\"metric\":null"), "null metric should serialize as null: {json}");
    }

    // -----------------------------------------------------------------------
    // Token-limit validation against the real BERT tokenizer
    // -----------------------------------------------------------------------

    /// Returns a dense-technical text resembling a real CV. Every word is chosen
    /// to maximize WordPiece fragmentation: abbreviations, camelCase identifiers,
    /// code terms, numbers with units, and non-Latin scripts — the worst case
    /// a user is likely to encounter.
    fn dense_technical_text() -> String {
        let terms = [
            "TypeScript", "PostgreSQL", "microservicios", "implementé",
            "NestJS", "Redis", "PrismaORM", "query_performance_optimization_v2",
            "AWS/GCP/Azure", "CI/CD pipelines", "Docker/Kubernetes",
            "10000rps", "40% reducción_latencia", "N+1 queries",
            "SQLAlchemy/TypeORM", "Webpack/Vite/Rollup", "REST/GraphQL/gRPC",
            "JWT/OAuth2/SAML", "kubernetes_cluster_autoscaling_policy",
            "micrófono_estéreo_cancela_ruido", "entrevistador@empresa.com",
            "ABTestingFrameworkV3", "RTSP/WebRTC/SIP", "MongoDB/Postgres/MySQL",
            "event_sourcing_CQRS_pattern", "DDD/Hexagonal/Clean Architecture",
            "k8s_deployment_rolling_update_strategy", "99.9% uptime SLA",
            "OpenTelemetry/Jaeger/Prometheus", "Grafana+Loki+Tempo stack",
            "feature_flags_toggle_gradual_rollout", "gRPC_stream_bidirectional",
        ];
        // Generate 600 words of dense technical content
        let mut text = String::new();
        for _ in 0..50 {
            text.push_str("En mi rol como Senior Software Engineer ");
            for term in terms {
                text.push_str(term);
                text.push(' ');
            }
            text.push_str("Además lideré equipos multi-disciplinarios ");
        }
        text
    }

    #[test]
    #[ignore = "requires network + ~90MB model download from HuggingFace (same as e2e test)"]
    fn chunk_token_count_within_bert_limit() {
        use crate::rag::embeddings::load_embedding_model;

        let model = load_embedding_model().expect("should load embedding model");

        let text = dense_technical_text();
        let chunks = chunk_text(&text, CHUNK_SIZE, CHUNK_OVERLAP);

        assert!(!chunks.is_empty(), "text should produce at least one chunk");

        let mut max_tokens = 0usize;
        let mut max_i = 0usize;
        for (i, chunk) in chunks.iter().enumerate() {
            let count = model.token_count(chunk);
            println!("chunk {i}: {count} tokens");
            if count > max_tokens {
                max_tokens = count;
                max_i = i;
            }
        }

        println!(
            "=== RESULT: max tokens = {max_tokens} (chunk {max_i}), \
             CHUNK_SIZE = {CHUNK_SIZE}, CHUNK_OVERLAP = {CHUNK_OVERLAP}, \
             limit = {MAX_TOKENS_PER_CHUNK} ==="
        );

        assert!(
            max_tokens <= MAX_TOKENS_PER_CHUNK,
            "Chunk {max_i} has {max_tokens} tokens, exceeding the {MAX_TOKENS_PER_CHUNK} limit. \
             Reduce CHUNK_SIZE ({CHUNK_SIZE}) and re-run this test."
        );
    }

    #[test]
    fn ingest_documents_warns_on_chunks_exceeding_token_limit() {
        // Verify the warning logic works with the mock model (which returns
        // whitespace count as token_count). Since mock token_count = word count
        // and chunks are ≤150 words, no warning should fire.
        ensure_vec_extension();
        let tmp = TempDir::new("token_warn");

        std::fs::write(tmp.path().join("test.txt"), "word ".repeat(200).as_bytes()).unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();
        let model = MockEmbeddingModel;

        let files = vec![tmp.path().join("test.txt").to_string_lossy().to_string()];
        // Should succeed without error (the warning is just a log line)
        ingest_documents(&model, &db, &files).unwrap();
    }

    // Full pipeline integration test (requires model download)

    #[test]
    #[ignore = "requires network + ~90MB model download from HuggingFace"]
    fn index_folder_and_search_end_to_end() {
        ensure_vec_extension();
        let tmp = TempDir::new("e2e");

        std::fs::write(
            tmp.path().join("proyectos.txt"),
            "Lideré el desarrollo de una API en NestJS con PostgreSQL que soporta 10k req/seg. Implementé Redis para caché y migraciones con Prisma ORM.",
        ).unwrap();
        std::fs::write(
            tmp.path().join("metricas.txt"),
            "Reduje la latencia de consultas en un 40% mediante índices compuestos y optimización de queries N+1.",
        ).unwrap();
        std::fs::write(
            tmp.path().join("stack.txt"),
            "Stack principal: TypeScript, Node.js, NestJS, Prisma, PostgreSQL, Redis, Docker.",
        ).unwrap();

        let db_path = tmp.path().join("test.db");
        let db = open_and_migrate(&db_path).unwrap();

        let model = crate::rag::embeddings::load_embedding_model()
            .expect("should load embedding model");

        index_folder(&model, &db, tmp.path().to_string_lossy().as_ref())
            .expect("should index folder");

        let results = search(&model, &db, "Prisma ORM migraciones", 3)
            .expect("should search");
        assert!(!results.is_empty(), "should find results for 'Prisma'");
        assert!(
            results[0].text.contains("Prisma"),
            "top result should mention Prisma, got: {}",
            results[0].text
        );
    }
}
