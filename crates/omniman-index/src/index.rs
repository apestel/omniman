use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::Context;
use omniman_core::{config::IndexConfig, types::Hit};
use tantivy::{
    collector::TopDocs,
    directory::MmapDirectory,
    query::{AllQuery, BooleanQuery, FuzzyTermQuery, Occur, Query, TermQuery},
    schema::IndexRecordOption,
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term,
};
use tantivy::schema::Value;
use tracing::{info, warn};
use walkdir::WalkDir;

use crate::schema::{self, Fields};

// Small heap shared by sweep / crawl / batch — Tantivy writers are single-threaded
// (writer_with_num_threads(1, …)) so peak RAM stays bounded at ~heap + segment buffers.
const WRITER_HEAP_BYTES: usize = 15_000_000;

#[derive(Debug, Default, Clone, Copy)]
pub struct SweepStats {
    pub scanned: usize,
    pub upserts: usize,
    pub deletes: usize,
}

pub struct FileIndex {
    index: Index,
    reader: IndexReader,
    fields: Fields,
    excludes: HashSet<String>,
    #[allow(dead_code)]
    index_dir: PathBuf,
}

impl FileIndex {
    pub fn open(index_dir: &Path, config: IndexConfig) -> anyhow::Result<Self> {
        std::fs::create_dir_all(index_dir)
            .with_context(|| format!("creating index dir {index_dir:?}"))?;

        let (schema, fields) = schema::build();
        let dir = MmapDirectory::open(index_dir)
            .with_context(|| format!("opening mmap dir {index_dir:?}"))?;

        let index = Index::open_or_create(dir, schema).context("open/create Tantivy index")?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("building index reader")?;

        let excludes = config.exclude_dirs.into_iter().collect();

        Ok(Self {
            index,
            reader,
            fields,
            excludes,
            index_dir: index_dir.to_owned(),
        })
    }

    /// Cheap startup pass: walk `root`, only touch the writer for new or modified files,
    /// remove paths that no longer exist on disk.  Replaces `crawl()` as the default
    /// startup work — typical idle restart adds zero documents and commits nothing.
    pub fn sweep(&self, root: &Path) -> anyhow::Result<SweepStats> {
        info!(?root, "starting mtime sweep");

        // Snapshot of (path → mtime) from the existing index.  Anything still in this
        // map after the walk is a stale entry (file deleted while the daemon was off).
        let mut indexed = self.snapshot_paths()?;
        let initial_size = indexed.len();

        let mut writer = self
            .index
            .writer_with_num_threads(1, WRITER_HEAP_BYTES)
            .context("creating sweep writer")?;

        let mut stats = SweepStats::default();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !self.is_excluded(e.path()))
        {
            let entry = match entry {
                Ok(e) => e,
                Err(ref err) if is_permission_denied_walk(err) => continue,
                Err(err) => {
                    warn!("walk error: {err}");
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let fs_mtime = mtime_secs(&meta);
            let path_str = path.to_string_lossy().to_string();
            stats.scanned += 1;

            let needs_index = match indexed.remove(&path_str) {
                None => true,                                  // new file
                Some(stored) if fs_mtime > stored => true,     // modified
                Some(_) => false,                               // unchanged
            };
            if needs_index {
                let term = Term::from_field_text(self.fields.path, &path_str);
                writer.delete_term(term);
                if let Some(doc) = self.make_document(path, &meta) {
                    writer.add_document(doc).ok();
                    stats.upserts += 1;
                }
            }
        }

        // Anything left in the map is a file that no longer exists.
        for stale_path in indexed.keys() {
            let term = Term::from_field_text(self.fields.path, stale_path);
            writer.delete_term(term);
            stats.deletes += 1;
        }

        if stats.upserts > 0 || stats.deletes > 0 {
            writer.commit().context("committing sweep")?;
        } else {
            // Drop the writer without committing — no-op restart with zero changes.
            drop(writer);
        }

        info!(
            scanned = stats.scanned,
            indexed_before = initial_size,
            upserts = stats.upserts,
            deletes = stats.deletes,
            "sweep complete"
        );
        Ok(stats)
    }

    /// Full re-crawl of `root` from scratch — wipes nothing but re-walks everything and
    /// re-indexes every file via delete-before-add.  Use this from the `Reindex` D-Bus
    /// method when the user wants a guaranteed-fresh state.
    pub fn crawl(&self, root: &Path) -> anyhow::Result<usize> {
        info!(?root, "starting full crawl");
        let mut writer = self
            .index
            .writer_with_num_threads(1, WRITER_HEAP_BYTES)
            .context("creating crawl writer")?;

        let mut count = 0usize;
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !self.is_excluded(e.path()))
        {
            let entry = match entry {
                Ok(e) => e,
                Err(ref err) if is_permission_denied_walk(err) => continue,
                Err(err) => {
                    warn!("walk error: {err}");
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let path = entry.path();
            let path_str = path.to_string_lossy().to_string();
            let term = Term::from_field_text(self.fields.path, &path_str);
            writer.delete_term(term);
            if let Some(doc) = self.make_document(path, &meta) {
                writer.add_document(doc).ok();
                count += 1;
            }
        }

        writer.commit().context("committing crawl")?;
        info!(count, "crawl complete");
        Ok(count)
    }

    /// Batch-apply file system changes: one writer, one commit, O(n) instead of O(n) writers.
    /// Called by the inotify watcher after accumulating events over a debounce window.
    pub fn apply_batch(&self, upserts: &[PathBuf], deletes: &[PathBuf]) -> anyhow::Result<()> {
        if upserts.is_empty() && deletes.is_empty() {
            return Ok(());
        }
        let mut writer = self.writer()?;
        for path in deletes {
            let term = Term::from_field_text(self.fields.path, &path.to_string_lossy());
            writer.delete_term(term);
        }
        for path in upserts {
            // Delete-before-add gives upsert semantics.
            let term = Term::from_field_text(self.fields.path, &path.to_string_lossy());
            writer.delete_term(term);
            let Ok(meta) = std::fs::metadata(path) else { continue };
            if !meta.is_file() {
                continue;
            }
            if let Some(doc) = self.make_document(path, &meta) {
                writer.add_document(doc).ok();
            }
        }
        writer.commit()?;
        Ok(())
    }

    /// Add or update a single file in the index.
    pub fn upsert(&self, path: &Path) -> anyhow::Result<()> {
        self.apply_batch(&[path.to_owned()], &[])
    }

    /// Remove a file from the index.
    pub fn remove(&self, path: &Path) -> anyhow::Result<()> {
        self.apply_batch(&[], &[path.to_owned()])
    }

    /// Search by query string — fuzzy match on filename + exact on parent path.
    pub fn search(&self, query: &str, limit: usize) -> anyhow::Result<Vec<Hit>> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }

        let searcher = self.reader.searcher();
        let query = self.build_query(query);

        let top_docs = searcher
            .search(&*query, &TopDocs::with_limit(limit))
            .context("searching index")?;

        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, addr) in top_docs {
            let doc: TantivyDocument = searcher.doc(addr).context("retrieving doc")?;
            let path = doc
                .get_first(self.fields.path)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let filename = doc
                .get_first(self.fields.filename)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            hits.push(Hit { path, filename, score: score as f64 });
        }
        Ok(hits)
    }

    /// Force an immediate index reader reload (useful in tests).
    pub fn reload(&self) -> anyhow::Result<()> {
        self.reader.reload().map_err(Into::into)
    }

    // ── private ──────────────────────────────────────────────────────────────

    fn writer(&self) -> anyhow::Result<IndexWriter> {
        self.index
            .writer_with_num_threads(1, WRITER_HEAP_BYTES)
            .context("creating index writer")
    }

    /// Snapshot (path → mtime) for every doc currently in the index.  Used by
    /// `sweep` to decide which files need re-indexing and which paths are stale.
    fn snapshot_paths(&self) -> anyhow::Result<HashMap<String, u64>> {
        let searcher = self.reader.searcher();
        let num_docs = searcher.num_docs() as usize;
        if num_docs == 0 {
            return Ok(HashMap::new());
        }
        // AllQuery + TopDocs(num_docs) iterates the whole index once.  Order doesn't
        // matter for our use case but TopDocs is the simplest way to enumerate.
        let top_docs = searcher
            .search(&AllQuery, &TopDocs::with_limit(num_docs))
            .context("enumerating index")?;
        let mut map = HashMap::with_capacity(top_docs.len());
        for (_, addr) in top_docs {
            let doc: TantivyDocument = searcher.doc(addr).context("retrieving doc")?;
            let path = doc
                .get_first(self.fields.path)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mtime = doc
                .get_first(self.fields.mtime)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if !path.is_empty() {
                map.insert(path, mtime);
            }
        }
        Ok(map)
    }

    fn make_document(&self, path: &Path, meta: &std::fs::Metadata) -> Option<TantivyDocument> {
        if !meta.is_file() {
            return None;
        }

        let path_str = path.to_string_lossy().to_string();
        let filename = path.file_name()?.to_string_lossy().to_string();
        let parent = path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let mtime = mtime_secs(meta);
        let size = meta.len();
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();

        let mut doc = TantivyDocument::default();
        doc.add_text(self.fields.path, &path_str);
        doc.add_text(self.fields.filename, &filename);
        doc.add_text(self.fields.parent, &parent);
        doc.add_text(self.fields.mime, &mime);
        doc.add_u64(self.fields.mtime, mtime);
        doc.add_u64(self.fields.size, size);
        Some(doc)
    }

    fn build_query(&self, query: &str) -> Box<dyn Query> {
        let term = Term::from_field_text(self.fields.filename, query);
        let fuzzy = FuzzyTermQuery::new(term, 1, true);

        if query.contains('/') {
            let path_term = Term::from_field_text(self.fields.path, query);
            let exact = TermQuery::new(path_term, IndexRecordOption::Basic);
            Box::new(BooleanQuery::new(vec![
                (Occur::Should, Box::new(fuzzy) as Box<dyn Query>),
                (Occur::Should, Box::new(exact) as Box<dyn Query>),
            ]))
        } else {
            Box::new(fuzzy)
        }
    }

    fn is_excluded(&self, path: &Path) -> bool {
        for c in path.components() {
            let s = c.as_os_str().to_string_lossy();
            if self.excludes.contains(s.as_ref()) {
                return true;
            }
        }
        false
    }
}

fn mtime_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn is_permission_denied_walk(err: &walkdir::Error) -> bool {
    err.io_error()
        .map(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use omniman_core::config::IndexConfig;
    use tempfile::TempDir;

    fn test_index(tmp: &TempDir) -> FileIndex {
        let config = IndexConfig {
            exclude_dirs: vec!["excluded".into(), "node_modules".into()],
        };
        FileIndex::open(&tmp.path().join("idx"), config).unwrap()
    }

    #[test]
    fn crawl_and_search_finds_file() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("rustacean.txt"), "ferris").unwrap();
        std::fs::write(home.join("notes.md"), "hello world").unwrap();

        let idx = test_index(&tmp);
        idx.crawl(&home).unwrap();
        idx.reload().unwrap();

        let hits = idx.search("rustacean", 10).unwrap();
        assert!(!hits.is_empty(), "should find rustacean.txt");
        assert_eq!(hits[0].filename, "rustacean.txt");
    }

    #[test]
    fn excluded_dir_not_indexed() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let excl = home.join("excluded");
        std::fs::create_dir_all(&excl).unwrap();
        std::fs::write(excl.join("hidden.txt"), "secret").unwrap();
        std::fs::write(home.join("visible.txt"), "visible").unwrap();

        let idx = test_index(&tmp);
        idx.crawl(&home).unwrap();
        idx.reload().unwrap();

        assert!(idx.search("hidden", 10).unwrap().is_empty(), "excluded file must not appear");
        assert!(!idx.search("visible", 10).unwrap().is_empty());
    }

    #[test]
    fn empty_query_returns_nothing() {
        let tmp = TempDir::new().unwrap();
        let idx = test_index(&tmp);
        assert!(idx.search("", 10).unwrap().is_empty());
    }

    #[test]
    fn upsert_then_remove() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("upsert_me.txt");
        std::fs::write(&file, "content").unwrap();

        let idx = test_index(&tmp);
        idx.upsert(&file).unwrap();
        idx.reload().unwrap();

        let hits = idx.search("upsert", 10).unwrap();
        assert!(!hits.is_empty());

        idx.remove(&file).unwrap();
    }

    #[test]
    fn sweep_indexes_new_files() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("alpha.txt"), "a").unwrap();
        std::fs::write(home.join("beta.txt"), "b").unwrap();

        let idx = test_index(&tmp);
        let stats = idx.sweep(&home).unwrap();
        idx.reload().unwrap();

        assert_eq!(stats.upserts, 2);
        assert_eq!(stats.deletes, 0);
        assert!(!idx.search("alpha", 10).unwrap().is_empty());
        assert!(!idx.search("beta", 10).unwrap().is_empty());
    }

    #[test]
    fn sweep_skips_unchanged_and_deletes_missing() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let alpha = home.join("alpha.txt");
        let beta = home.join("beta.txt");
        std::fs::write(&alpha, "a").unwrap();
        std::fs::write(&beta, "b").unwrap();

        let idx = test_index(&tmp);
        idx.sweep(&home).unwrap();
        idx.reload().unwrap();

        // Second sweep with no FS changes → zero work.
        let stats = idx.sweep(&home).unwrap();
        assert_eq!(stats.upserts, 0);
        assert_eq!(stats.deletes, 0);
        assert_eq!(stats.scanned, 2);

        // Delete one file and re-sweep — the stale entry should be removed.
        std::fs::remove_file(&beta).unwrap();
        let stats = idx.sweep(&home).unwrap();
        idx.reload().unwrap();
        assert_eq!(stats.upserts, 0);
        assert_eq!(stats.deletes, 1);
        assert!(idx.search("beta", 10).unwrap().is_empty());
        assert!(!idx.search("alpha", 10).unwrap().is_empty());
    }

    #[test]
    fn sweep_reindexes_modified_file() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let gamma = home.join("gamma.txt");
        std::fs::write(&gamma, "v1").unwrap();

        let idx = test_index(&tmp);
        idx.sweep(&home).unwrap();
        idx.reload().unwrap();

        // Sleep past 1 s and rewrite so the OS bumps mtime to a strictly greater
        // second-resolution timestamp than what's stored.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&gamma, "v2-and-longer").unwrap();

        let stats = idx.sweep(&home).unwrap();
        assert_eq!(stats.upserts, 1, "modified file should be re-indexed");
        assert_eq!(stats.deletes, 0);
    }
}
