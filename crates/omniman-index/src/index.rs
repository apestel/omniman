use std::{
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::Context;
use omniman_core::{config::IndexConfig, types::Hit};
use tantivy::{
    collector::TopDocs,
    directory::MmapDirectory,
    query::{BooleanQuery, FuzzyTermQuery, Occur, Query, TermQuery},
    schema::IndexRecordOption,
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term,
};
use tantivy::schema::Value;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

use crate::schema::{self, Fields};

// Large heap only for the initial crawl (kept as a single allocation, released on drop).
const CRAWL_HEAP_MB: usize = 50_000_000;
// Small heap for incremental upsert/remove/batch — one writer at a time.
const WRITER_HEAP_MB: usize = 15_000_000;

pub struct FileIndex {
    index: Index,
    reader: IndexReader,
    fields: Fields,
    config: IndexConfig,
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

        Ok(Self {
            index,
            reader,
            fields,
            config,
            index_dir: index_dir.to_owned(),
        })
    }

    /// Full initial crawl of `root` — single-threaded walk, streams docs directly into the
    /// writer without holding them all in RAM. Writer auto-flushes segments to disk when its
    /// internal buffer (CRAWL_HEAP_MB) fills, so peak RAM stays bounded regardless of tree size.
    pub fn crawl(&self, root: &Path) -> anyhow::Result<usize> {
        info!(?root, "starting initial crawl");
        let mut writer = self.index.writer(CRAWL_HEAP_MB).context("creating crawl writer")?;

        let mut count = 0usize;
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !self.is_excluded(e.path()))
        {
            let entry = match entry {
                Ok(e) => e,
                Err(ref err) if is_permission_denied_walk(err) => {
                    debug!("skipping (permission denied): {:?}", err.path());
                    continue;
                }
                Err(err) => {
                    warn!("walk error: {err}");
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            if let Some(doc) = self.make_document(entry.path()) {
                writer.add_document(doc).ok();
                count += 1;
            }
        }

        writer.commit().context("committing initial crawl")?;
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
            if let Some(doc) = self.make_document(path) {
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
            .writer(WRITER_HEAP_MB)
            .context("creating index writer")
    }

    fn make_document(&self, path: &Path) -> Option<TantivyDocument> {
        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                debug!("skipping (permission denied): {path:?}");
                return None;
            }
            Err(_) => return None,
        };
        if !meta.is_file() {
            return None;
        }

        let path_str = path.to_string_lossy().to_string();
        let filename = path.file_name()?.to_string_lossy().to_string();
        let parent = path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
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
        path.components().any(|c| {
            let s = c.as_os_str().to_string_lossy();
            self.config.exclude_dirs.iter().any(|ex| s == ex.as_str())
        })
    }
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
}
