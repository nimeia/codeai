use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use blake3::Hasher;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::embedding;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexRunMode {
    Full,
    Incremental,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexJob {
    pub project_root: PathBuf,
    pub cache_dir: PathBuf,
    pub metadata_path: PathBuf,
    pub vector_path: PathBuf,
    pub schema_version: u32,
    pub mode: IndexRunMode,
}

impl IndexJob {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        let project_root = project_root.into();
        let workdir = project_root.join(".code-nav");
        Self {
            project_root,
            cache_dir: workdir.join("ir.cache"),
            metadata_path: workdir.join("metadata.db"),
            vector_path: workdir.join("hnsw.index"),
            schema_version: 1,
            mode: IndexRunMode::Full,
        }
    }

    pub fn with_mode(mut self, mode: IndexRunMode) -> Self {
        self.mode = mode;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexReport {
    pub files_total: usize,
    pub files_indexed: usize,
    pub symbols_indexed: usize,
    pub embeddings_written: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexProgress {
    pub files_completed: usize,
    pub files_total: usize,
    pub current_file: Option<String>,
}

impl IndexProgress {
    pub fn percent(&self) -> Option<f32> {
        if self.files_total == 0 {
            return None;
        }
        Some((self.files_completed as f32 / self.files_total as f32) * 100.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileRecord {
    pub path: String,
    pub lang: String,
    pub digest: String,
    pub size: u64,
    pub mtime: i64,
    pub version: u64,
    pub is_deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IrRecord {
    pub file_path: String,
    pub lang: String,
    pub schema_ver: u32,
    pub ir_hash: String,
    pub stored_path: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SymbolRecord {
    pub file_path: String,
    pub name: String,
    pub kind: String,
    pub fqname: String,
    pub span_start: u64,
    pub span_end: u64,
    pub doc: Option<String>,
    pub version: u64,
    pub is_deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexCursor {
    pub component: String,
    pub last_offset: Option<String>,
    pub schema_ver: u32,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetadataStore {
    pub files: Vec<FileRecord>,
    pub ir_blobs: Vec<IrRecord>,
    pub symbols: Vec<SymbolRecord>,
    pub index_cursors: Vec<IndexCursor>,
}

impl MetadataStore {
    fn load(path: &Path) -> Result<Self> {
        if path.exists() {
            let data = fs::read(path)?;
            return Ok(serde_json::from_slice(&data).context("invalid metadata.db content")?);
        }
        Ok(Self::default())
    }

    fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create metadata dir {}", parent.display()))?;
        }
        let content = serde_json::to_vec_pretty(self)?;
        fs::write(path, content)
            .with_context(|| format!("failed to write metadata to {}", path.display()))
    }

    fn reset(&mut self) {
        self.files.clear();
        self.ir_blobs.clear();
        self.symbols.clear();
        self.index_cursors.clear();
    }

    fn upsert_file(&mut self, file: FileRecord) {
        if let Some(existing) = self.files.iter_mut().find(|f| f.path == file.path) {
            *existing = file;
        } else {
            self.files.push(file);
        }
    }

    fn upsert_ir(&mut self, record: IrRecord) {
        if let Some(existing) = self
            .ir_blobs
            .iter_mut()
            .find(|blob| blob.file_path == record.file_path)
        {
            *existing = record;
        } else {
            self.ir_blobs.push(record);
        }
    }

    fn replace_symbols(&mut self, file_path: &str, symbols: Vec<SymbolRecord>) {
        self.symbols.retain(|sym| sym.file_path != file_path);
        self.symbols.extend(symbols);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VectorEntry {
    pub file_path: String,
    pub symbol: String,
    pub lang: String,
    pub vector: Vec<f32>,
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VectorIndex {
    pub entries: BTreeMap<String, VectorEntry>,
}

impl VectorIndex {
    fn load(path: &Path) -> Result<Self> {
        if path.exists() {
            let data = fs::read(path)?;
            let map: BTreeMap<String, VectorEntry> =
                serde_json::from_slice(&data).context("invalid vector index content")?;
            return Ok(Self { entries: map });
        }
        Ok(Self::default())
    }

    fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create vector dir {}", parent.display()))?;
        }
        let content = serde_json::to_vec_pretty(&self.entries)?;
        fs::write(path, content)
            .with_context(|| format!("failed to write vector index to {}", path.display()))
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn replace_for_file(&mut self, file_path: &str, symbols: &[SymbolRecord]) -> usize {
        self.entries.retain(|_, entry| entry.file_path != file_path);

        let mut inserted = 0;
        for symbol in symbols.iter().filter(|sym| !sym.is_deleted) {
            let vector = embedding::embed(&symbol.fqname);
            let key = format!("{}::{}", file_path, symbol.fqname);
            self.entries.insert(
                key,
                VectorEntry {
                    file_path: file_path.to_string(),
                    symbol: symbol.fqname.clone(),
                    lang: symbol.kind.clone(),
                    vector,
                    version: symbol.version,
                },
            );
            inserted += 1;
        }
        inserted
    }

    fn remove_for_file(&mut self, file_path: &str) {
        self.entries.retain(|_, entry| entry.file_path != file_path);
    }
}

pub fn run_with_progress<F>(job: IndexJob, mut callback: F) -> Result<IndexReport>
where
    F: FnMut(IndexProgress),
{
    let mut metadata = MetadataStore::load(&job.metadata_path)?;
    let mut vector_index = VectorIndex::load(&job.vector_path)?;

    if matches!(job.mode, IndexRunMode::Full) {
        metadata.reset();
        vector_index.clear();
    }

    fs::create_dir_all(&job.cache_dir).with_context(|| {
        format!(
            "failed to create IR cache directory {}",
            job.cache_dir.display()
        )
    })?;
    fs::create_dir_all(job.metadata_path.parent().unwrap_or_else(|| Path::new(".")))
        .with_context(|| "failed to create metadata parent directory".to_string())?;

    let files: Vec<PathBuf> = WalkDir::new(&job.project_root)
        .into_iter()
        .filter_entry(|entry| !is_ignored(entry.path()))
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .collect();

    let mut report = IndexReport {
        files_total: files.len(),
        ..Default::default()
    };

    let mut seen_files = Vec::new();
    for (idx, file_path) in files.iter().enumerate() {
        let relative = file_path
            .strip_prefix(&job.project_root)
            .unwrap_or(file_path)
            .to_path_buf();
        callback(IndexProgress {
            files_completed: idx,
            files_total: report.files_total,
            current_file: Some(relative.display().to_string()),
        });

        let (file_record, ir_record, symbols) = process_file(
            file_path,
            &relative,
            &job.cache_dir,
            job.schema_version,
            metadata
                .files
                .iter()
                .find(|f| f.path == relative.display().to_string()),
        )?;

        metadata.upsert_file(file_record.clone());
        metadata.upsert_ir(ir_record);
        metadata.replace_symbols(&file_record.path, symbols.clone());

        let embeddings = vector_index.replace_for_file(&file_record.path, &symbols);

        report.files_indexed += 1;
        report.symbols_indexed += symbols.len();
        report.embeddings_written += embeddings;
        seen_files.push(file_record.path.clone());
    }

    if matches!(job.mode, IndexRunMode::Incremental) {
        for file in metadata.files.iter_mut() {
            if !seen_files.contains(&file.path) && !file.is_deleted {
                file.is_deleted = true;
                file.version += 1;
                vector_index.remove_for_file(&file.path);
                metadata
                    .symbols
                    .iter_mut()
                    .filter(|sym| sym.file_path == file.path)
                    .for_each(|sym| {
                        sym.is_deleted = true;
                        sym.version += 1;
                    });
            }
        }
    }

    callback(IndexProgress {
        files_completed: report.files_total,
        files_total: report.files_total,
        current_file: None,
    });

    metadata.save(&job.metadata_path)?;
    vector_index.save(&job.vector_path)?;
    Ok(report)
}

fn process_file(
    absolute: &Path,
    relative: &Path,
    cache_dir: &Path,
    schema_ver: u32,
    previous: Option<&FileRecord>,
) -> Result<(FileRecord, IrRecord, Vec<SymbolRecord>)> {
    let content = fs::read_to_string(absolute)
        .with_context(|| format!("failed to read file {}", absolute.display()))?;
    let lang = detect_language(absolute);
    let digest = hash_contents(&content);
    let metadata = absolute.metadata()?;
    let mtime = metadata
        .modified()
        .unwrap_or(SystemTime::now())
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let mut file_record = FileRecord {
        path: relative.display().to_string(),
        lang: lang.to_string(),
        digest: digest.clone(),
        size: metadata.len(),
        mtime,
        version: previous.map(|f| f.version + 1).unwrap_or(1),
        is_deleted: false,
    };

    if let Some(prev) = previous {
        if prev.digest == digest {
            file_record.version = prev.version;
        }
    }

    let symbols = extract_symbols(
        &relative.display().to_string(),
        &content,
        &lang,
        file_record.version,
    );

    let ir_path = cache_dir
        .join(format!("v{schema_ver}"))
        .join(&lang)
        .join(format!("{digest}.json"));
    let ir_record = IrRecord {
        file_path: file_record.path.clone(),
        lang: lang.to_string(),
        schema_ver,
        ir_hash: digest,
        stored_path: ir_path
            .strip_prefix(cache_dir.parent().unwrap_or(Path::new("")))
            .unwrap_or(&ir_path)
            .display()
            .to_string(),
        created_at: Utc::now().timestamp(),
    };

    let ir_body = serde_json::json!({
        "file": {
            "path": file_record.path,
            "lang": file_record.lang,
            "size": file_record.size,
            "mtime": file_record.mtime,
            "version": file_record.version,
        },
        "symbols": symbols.iter().map(|sym| serde_json::json!({
            "name": sym.name,
            "kind": sym.kind,
            "fqname": sym.fqname,
            "span_start": sym.span_start,
            "span_end": sym.span_end,
            "doc": sym.doc,
        })).collect::<Vec<_>>(),
    });
    if let Some(parent) = ir_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create IR dir {}", parent.display()))?;
    }
    fs::write(&ir_path, serde_json::to_vec_pretty(&ir_body)?)
        .with_context(|| format!("failed to write IR cache {}", ir_path.display()))?;

    Ok((file_record, ir_record, symbols))
}

fn hash_contents(content: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(content.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn detect_language(path: &Path) -> &str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
    {
        "rs" => "rust",
        "py" => "python",
        "js" => "javascript",
        "ts" => "typescript",
        "java" => "java",
        "go" => "go",
        "cpp" | "cxx" | "cc" => "cpp",
        "c" => "c",
        "md" => "markdown",
        _ => "text",
    }
}

fn extract_symbols(file_path: &str, content: &str, lang: &str, version: u64) -> Vec<SymbolRecord> {
    let mut symbols = Vec::new();
    let mut line_number: u64 = 1;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            line_number += 1;
            continue;
        }
        let name = trimmed
            .split_whitespace()
            .next()
            .unwrap_or_else(|| file_path)
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
            .to_string();
        let fqname = format!("{file_path}::{name}");
        symbols.push(SymbolRecord {
            file_path: file_path.to_string(),
            name: name.clone(),
            kind: lang.to_string(),
            fqname,
            span_start: line_number,
            span_end: line_number,
            doc: None,
            version,
            is_deleted: false,
        });
        line_number += 1;
        if symbols.len() >= 8 {
            break;
        }
    }

    if symbols.is_empty() {
        symbols.push(SymbolRecord {
            file_path: file_path.to_string(),
            name: Path::new(file_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("root")
                .to_string(),
            kind: lang.to_string(),
            fqname: format!("{file_path}::root"),
            span_start: 1,
            span_end: 1,
            doc: Some("auto-generated placeholder".to_string()),
            version,
            is_deleted: false,
        });
    }

    symbols
}

fn is_ignored(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        matches!(name, ".git" | ".code-nav" | "target" | "node_modules")
    } else {
        false
    }
}
