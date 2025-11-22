use std::{
    collections::hash_map::DefaultHasher,
    fmt, fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use clap::{builder::NonEmptyStringValueParser, Args, Subcommand, ValueEnum};
use code_nav_protocol::{
    IndexMode, ProjectAddRequest, ProjectListRequest, ProjectRef, ProjectRemoveRequest,
    ProjectRestartRequest, Request, StatusRequest, StatusResponse, WorkerStatus,
};
use dirs::home_dir;
use serde::{Deserialize, Serialize};

use crate::formatter;
use crate::CliContext;

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Register a project and schedule worker initialization.
    Add(ProjectAddArgs),
    /// Remove a project from the registry.
    Remove(ProjectRemoveArgs),
    /// List registered projects and their runtime state.
    List(ProjectListArgs),
    /// Show detailed runtime information for a single project.
    Status(ProjectStatusArgs),
    /// Restart one or multiple project workers.
    Restart(ProjectRestartArgs),
}

pub fn run(ctx: &CliContext, cmd: ProjectCommand) -> Result<()> {
    match cmd {
        ProjectCommand::Add(args) => handle_add(ctx, args),
        ProjectCommand::Remove(args) => handle_remove(ctx, args),
        ProjectCommand::List(args) => handle_list(ctx, args),
        ProjectCommand::Status(args) => handle_status(ctx, args),
        ProjectCommand::Restart(args) => handle_restart(ctx, args),
    }
}

#[derive(Debug, Args)]
pub struct ProjectAddArgs {
    /// Path to the project root directory.
    #[arg(long = "project", value_name = "PATH")]
    pub project: PathBuf,
    /// Optional custom project identifier.
    #[arg(long = "id")]
    pub id: Option<String>,
    /// Override the default runtime directory (~/.code-nav).
    #[arg(long = "runtime-dir", value_name = "PATH")]
    pub runtime_dir: Option<PathBuf>,
    /// Mark the project for automatic start when master launches.
    #[arg(long)]
    pub autostart: bool,
    /// Enable filesystem watch after indexing completes.
    #[arg(long)]
    pub watch: bool,
    /// Preferred indexing mode for the worker.
    #[arg(long = "index-mode", default_value_t = IndexModeArg::Auto, value_enum)]
    pub index_mode: IndexModeArg,
    /// Preferred embedding/model backend for the project.
    #[arg(long = "model")]
    pub model: Option<String>,
    /// Output the registration payload as JSON instead of text.
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ProjectRemoveArgs {
    /// Path or identifier of the project to remove.
    #[arg(long = "project", value_name = "PATH|ID", value_parser = NonEmptyStringValueParser::new())]
    pub project: String,
    /// Override the default runtime directory (~/.code-nav).
    #[arg(long = "runtime-dir", value_name = "PATH")]
    pub runtime_dir: Option<PathBuf>,
    /// Force stop the worker if it does not shut down cleanly.
    #[arg(long = "force")]
    pub force: bool,
    /// Keep the .code-nav data directory instead of deleting it.
    #[arg(long = "keep-data")]
    pub keep_data: bool,
    /// Grace period in seconds before forcing termination.
    #[arg(long = "grace", value_name = "SECONDS", default_value_t = 10)]
    pub grace: u64,
    /// Emit JSON output instead of text.
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ProjectListArgs {
    /// Output format for the list command.
    #[arg(long = "format", value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
    /// Override the default runtime directory (~/.code-nav).
    #[arg(long = "runtime-dir", value_name = "PATH")]
    pub runtime_dir: Option<PathBuf>,
    /// Filter projects by runtime status.
    #[arg(long = "status", value_enum)]
    pub status: Option<ProjectRuntimeFilter>,
    /// Include extended debugging columns.
    #[arg(long = "verbose")]
    pub verbose: bool,
}

#[derive(Debug, Args)]
pub struct ProjectStatusArgs {
    /// Path or identifier of the project to inspect.
    #[arg(long = "project", value_name = "PATH|ID", value_parser = NonEmptyStringValueParser::new())]
    pub project: String,
    /// Override the default runtime directory (~/.code-nav).
    #[arg(long = "runtime-dir", value_name = "PATH")]
    pub runtime_dir: Option<PathBuf>,
    /// Continuously refresh the status output at the given interval.
    #[arg(long = "watch", value_name = "SECONDS")]
    pub watch: Option<u64>,
    /// Emit JSON output.
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ProjectRestartArgs {
    /// One or multiple project references to restart.
    #[arg(
        long = "project",
        value_name = "PATH|ID",
        num_args = 1..,
        value_parser = NonEmptyStringValueParser::new()
    )]
    pub project: Vec<String>,
    /// Override the default runtime directory (~/.code-nav).
    #[arg(long = "runtime-dir", value_name = "PATH")]
    pub runtime_dir: Option<PathBuf>,
    /// Wait for the restart workflow to finish.
    #[arg(long = "wait")]
    pub wait: bool,
    /// Force terminate workers that ignore shutdown.
    #[arg(long = "force")]
    pub force: bool,
    /// Grace period in seconds before force stopping workers.
    #[arg(long = "grace", value_name = "SECONDS")]
    pub grace: Option<u64>,
    /// Maximum number of seconds to wait when --wait is supplied.
    #[arg(long = "timeout", value_name = "SECONDS")]
    pub timeout: Option<u64>,
    /// Reason that explains why the restart was triggered.
    #[arg(long = "reason")]
    pub reason: Option<String>,
    /// Emit JSON output.
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexModeArg {
    Full,
    Incremental,
    Auto,
}

impl Default for IndexModeArg {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Table,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectRuntimeFilter {
    Running,
    Stopped,
    Failed,
}

impl fmt::Display for ProjectRuntimeFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            ProjectRuntimeFilter::Running => "running",
            ProjectRuntimeFilter::Stopped => "stopped",
            ProjectRuntimeFilter::Failed => "failed",
        };
        write!(f, "{}", value)
    }
}

fn to_protocol_index_mode(mode: IndexModeArg) -> IndexMode {
    match mode {
        IndexModeArg::Full => IndexMode::Full,
        IndexModeArg::Incremental => IndexMode::Incremental,
        IndexModeArg::Auto => IndexMode::Auto,
    }
}

fn to_protocol_project_ref(reference: &str) -> ProjectRef {
    if Path::new(reference).exists() {
        ProjectRef::Path(reference.to_string())
    } else {
        ProjectRef::Id(reference.to_string())
    }
}

fn to_protocol_project_runtime_filter(
    filter: ProjectRuntimeFilter,
) -> code_nav_protocol::ProjectRuntimeFilter {
    match filter {
        ProjectRuntimeFilter::Running => code_nav_protocol::ProjectRuntimeFilter::Running,
        ProjectRuntimeFilter::Stopped => code_nav_protocol::ProjectRuntimeFilter::Stopped,
        ProjectRuntimeFilter::Failed => code_nav_protocol::ProjectRuntimeFilter::Failed,
    }
}

const REGISTRY_VERSION: u32 = 1;

fn handle_add(ctx: &CliContext, args: ProjectAddArgs) -> Result<()> {
    let project_root = normalize_project_path(&args.project)?;
    let mut registry = ProjectRegistry::load(args.runtime_dir.as_deref())?;

    if registry.contains_path(&project_root) {
        bail!("project {} is already registered", project_root.display());
    }

    let project_id = if let Some(custom) = args.id.clone() {
        custom
    } else {
        generate_project_id(&project_root)
    };

    if registry.contains_id(&project_id) {
        bail!("project id {project_id} already exists; supply --id with a unique value");
    }

    let request = Request::ProjectAdd(ProjectAddRequest {
        project_root: project_root.clone(),
        id: args.id.clone(),
        autostart: args.autostart,
        watch: args.watch,
        index_mode: to_protocol_index_mode(args.index_mode),
        model: args.model.clone(),
    });
    let _response = ctx.client.send(&request)?; // Send request through new RpcClient

    let entry = ProjectEntry {
        project_id: project_id.clone(),
        project_root: project_root.clone(),
        autostart: args.autostart,
        watch: args.watch,
        index_mode: args.index_mode,
        model: args.model.clone(),
        last_running: false,
        last_state: Some("pending".to_string()),
    };
    let snapshot = ProjectSnapshot::from_project_entry(&entry);
    registry.add(entry);
    registry.persist()?;

    if args.json {
        formatter::print_line(&serde_json::to_string_pretty(&snapshot)?);
    } else {
        formatter::print_line(&format!(
            "Project registered: {} ({})",
            snapshot.project_id,
            snapshot.project_root.display()
        ));
        formatter::print_line("Worker: initializing (pending)");
        formatter::print_line("Add request sent to master daemon.");
    }
    Ok(())
}

fn handle_remove(ctx: &CliContext, args: ProjectRemoveArgs) -> Result<()> {
    let request = Request::ProjectRemove(ProjectRemoveRequest {
        project: to_protocol_project_ref(&args.project),
        force: args.force,
        keep_data: args.keep_data,
        grace_sec: args.grace,
    });
    let _response = ctx.client.send(&request)?; // Send request through new RpcClient

    let mut registry = ProjectRegistry::load(args.runtime_dir.as_deref())?;
    let removed = registry
        .remove(&args.project)
        .with_context(|| format!("no project matched reference '{}'", args.project))?;
    registry.persist()?;

    let snapshot = ProjectSnapshot::from_project_entry(&removed);
    if args.json {
        formatter::print_line(&serde_json::to_string_pretty(&snapshot)?);
    } else {
        formatter::print_line(&format!(
            "Project removed: {} ({})",
            removed.project_id,
            removed.project_root.display()
        ));
        formatter::print_line("Remove request sent to master daemon.");
    }
    Ok(())
}

fn handle_list(ctx: &CliContext, args: ProjectListArgs) -> Result<()> {
    let request = Request::ProjectList(ProjectListRequest {
        status: args.status.map(to_protocol_project_runtime_filter),
    });
    let _response = ctx.client.send(&request)?; // Send request through new RpcClient
    formatter::print_line("List request sent to master daemon. Showing local cache.");

    let registry = ProjectRegistry::load(args.runtime_dir.as_deref())?;
    let mut entries = registry
        .entries()
        .iter()
        .map(ProjectSnapshot::from_project_entry)
        .collect::<Vec<_>>();
    if let Some(filter) = args.status {
        entries.retain(|entry| filter.matches_state(&entry.state));
    }
    entries.sort_by(|a, b| a.project_id.cmp(&b.project_id));

    if args.format == OutputFormat::Json {
        formatter::print_line(&serde_json::to_string_pretty(&ProjectListOutput {
            total: entries.len(),
            projects: entries,
        })?);
    } else {
        print_project_table(&entries, args.verbose);
    }
    Ok(())
}

fn handle_status(ctx: &CliContext, args: ProjectStatusArgs) -> Result<()> {
    let project_ref = to_protocol_project_ref(&args.project);

    if let Some(interval) = args.watch {
        watch_worker_status(ctx, project_ref, interval, args.json)?;
        return Ok(());
    }

    let worker_status = fetch_worker_status(ctx, project_ref)?;
    render_worker_status(worker_status, args.json)
}

fn print_project_table(entries: &[ProjectSnapshot], verbose: bool) {
    let mut rows = Vec::with_capacity(entries.len() + 1);
    let mut header = vec![
        "ID".to_string(),
        "Path".to_string(),
        "State".to_string(),
        "Autostart".to_string(),
        "Watch".to_string(),
    ];
    if verbose {
        header.push("Index Mode".to_string());
        header.push("Model".to_string());
    }
    rows.push(header);

    for entry in entries {
        let mut row = vec![
            entry.project_id.clone(),
            entry.project_root.display().to_string(),
            entry.state.clone(),
            entry.autostart.to_string(),
            entry.watch.to_string(),
        ];
        if verbose {
            row.push(format!("{:?}", entry.index_mode));
            row.push(entry.model.clone().unwrap_or_else(|| "-".into()));
        }
        rows.push(row);
    }

    let column_count = rows[0].len();
    let mut widths = vec![0usize; column_count];
    for row in &rows {
        for (idx, cell) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(cell.len());
        }
    }

    for row in rows {
        let formatted = row
            .iter()
            .enumerate()
            .map(|(idx, cell)| format!("{:<width$}", cell, width = widths[idx]))
            .collect::<Vec<_>>()
            .join("  ");
        formatter::print_line(&formatted);
    }
}

fn fetch_worker_status(ctx: &CliContext, project_ref: ProjectRef) -> Result<WorkerStatus> {
    let request = Request::Status(StatusRequest {
        target: Some(project_ref),
    });
    let response = ctx.client.send(&request)?; // Send request through new RpcClient

    if let code_nav_protocol::Response::Status(StatusResponse::Worker(worker_status)) = response {
        Ok(worker_status)
    } else {
        bail!("Error: received unexpected response type from server");
    }
}

fn render_worker_status(status: WorkerStatus, json: bool) -> Result<()> {
    if json {
        formatter::print_line(&serde_json::to_string_pretty(&status)?);
    } else {
        formatter::print_worker_status(status);
    }
    Ok(())
}

fn watch_worker_status(
    ctx: &CliContext,
    project_ref: ProjectRef,
    interval_secs: u64,
    json: bool,
) -> Result<()> {
    let interval_secs = interval_secs.max(1);
    let sleep_duration = Duration::from_secs(interval_secs);

    if !json {
        formatter::print_line(&format!(
            "Watching project status every {}s (press Ctrl+C to stop)...",
            interval_secs
        ));
    }

    loop {
        let worker_status = fetch_worker_status(ctx, project_ref.clone())?;
        render_worker_status(worker_status, json)?;
        std::thread::sleep(sleep_duration);
    }
}

fn handle_restart(ctx: &CliContext, args: ProjectRestartArgs) -> Result<()> {
    let request = Request::ProjectRestart(ProjectRestartRequest {
        projects: args
            .project
            .iter()
            .map(|p| to_protocol_project_ref(p))
            .collect(),
        wait: args.wait,
        force: args.force,
        grace_sec: args.grace,
        timeout_sec: args.timeout,
        reason: args.reason.clone(),
    });
    let _response = ctx.client.send(&request)?; // Send request through new RpcClient

    let mut registry = ProjectRegistry::load(args.runtime_dir.as_deref())?;
    let mut updated = Vec::new();
    for reference in &args.project {
        let entry = registry
            .find_mut(reference)
            .with_context(|| format!("no project matched reference '{reference}'"))?;
        entry.last_running = false;
        entry.last_state = Some("restarting".to_string());
        updated.push(ProjectSnapshot::from_project_entry(entry));
    }
    registry.persist()?;

    if args.json {
        formatter::print_line(&serde_json::to_string_pretty(&RestartOutput {
            projects: updated,
        })?);
    } else {
        formatter::print_line(&format!(
            "Restart scheduled for {} project(s)",
            args.project.len()
        ));
        for project in &args.project {
            formatter::print_line(&format!("  - {project}"));
        }
        formatter::print_line("Restart request sent to master daemon.");
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegistryFile {
    version: u32,
    #[serde(default)]
    projects: Vec<ProjectEntry>,
}

impl Default for RegistryFile {
    fn default() -> Self {
        Self {
            version: REGISTRY_VERSION,
            projects: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectEntry {
    project_id: String,
    project_root: PathBuf,
    #[serde(default)]
    autostart: bool,
    #[serde(default)]
    watch: bool,
    #[serde(default)]
    index_mode: IndexModeArg,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    last_running: bool,
    #[serde(default)]
    last_state: Option<String>,
}

impl ProjectEntry {
    fn state_label(&self) -> String {
        if let Some(state) = &self.last_state {
            return state.clone();
        }
        if self.last_running {
            "running".to_string()
        } else {
            "stopped".to_string()
        }
    }
}

struct ProjectRegistry {
    path: PathBuf,
    file: RegistryFile,
}

impl ProjectRegistry {
    fn load(runtime_dir_override: Option<&Path>) -> Result<Self> {
        let path = registry_path(runtime_dir_override)?;
        if !path.exists() {
            let file = RegistryFile::default();
            let json = serde_json::to_vec_pretty(&file)?;
            fs::write(&path, json)?;
            return Ok(Self { path, file });
        }
        let content =
            fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        let file = if content.is_empty() {
            RegistryFile::default()
        } else {
            serde_json::from_slice(&content).with_context(|| {
                format!(
                    "invalid registry file {}; try removing it and rerunning",
                    path.display()
                )
            })?
        };
        Ok(Self { path, file })
    }

    fn persist(&self) -> Result<()> {
        let json = serde_json::to_vec_pretty(&self.file)?;
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, &json)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    fn entries(&self) -> &[ProjectEntry] {
        &self.file.projects
    }

    fn add(&mut self, entry: ProjectEntry) {
        self.file.projects.push(entry);
    }

    fn contains_path(&self, path: &Path) -> bool {
        self.file
            .projects
            .iter()
            .any(|entry| entry.project_root == path)
    }

    fn contains_id(&self, id: &str) -> bool {
        self.file
            .projects
            .iter()
            .any(|entry| entry.project_id == id)
    }

    fn find_mut(&mut self, reference: &str) -> Option<&mut ProjectEntry> {
        self.file
            .projects
            .iter_mut()
            .find(|entry| entry.matches(reference))
    }

    fn remove(&mut self, reference: &str) -> Option<ProjectEntry> {
        if let Some(idx) = self
            .file
            .projects
            .iter()
            .position(|entry| entry.matches(reference))
        {
            Some(self.file.projects.remove(idx))
        } else {
            None
        }
    }
}

impl ProjectEntry {
    fn matches(&self, reference: &str) -> bool {
        if self.project_id == reference {
            return true;
        }
        if let Ok(candidate) = normalize_project_path_optional(reference) {
            if let Some(candidate) = candidate {
                return candidate == self.project_root;
            }
        }
        self.project_root.to_string_lossy() == reference
    }
}

fn normalize_project_path(path: &Path) -> Result<PathBuf> {
    if !path.exists() {
        bail!("project path {} does not exist", path.display());
    }
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("failed to canonicalize path {}", path.display()))?;
    if !canonical.is_dir() {
        bail!("{} is not a directory", canonical.display());
    }
    Ok(canonical)
}

fn normalize_project_path_optional(reference: &str) -> Result<Option<PathBuf>> {
    let path = Path::new(reference);
    if path.exists() {
        return Ok(Some(fs::canonicalize(path)?));
    }
    Ok(None)
}

fn registry_path(runtime_dir_override: Option<&Path>) -> Result<PathBuf> {
    let runtime_dir = if let Some(custom) = runtime_dir_override {
        custom.to_path_buf()
    } else {
        let home = home_dir().ok_or_else(|| anyhow!("failed to determine home directory"))?;
        home.join(".code-nav")
    };
    let projects_dir = runtime_dir.join("projects");
    fs::create_dir_all(&projects_dir)?;
    Ok(projects_dir.join("registry.json"))
}

fn generate_project_id(path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[derive(Debug, Serialize)]
struct ProjectSnapshot {
    project_id: String,
    project_root: PathBuf,
    autostart: bool,
    watch: bool,
    index_mode: IndexModeArg,
    model: Option<String>,
    state: String,
}

impl ProjectSnapshot {
    fn from_project_entry(entry: &ProjectEntry) -> Self {
        Self {
            project_id: entry.project_id.clone(),
            project_root: entry.project_root.clone(),
            autostart: entry.autostart,
            watch: entry.watch,
            index_mode: entry.index_mode,
            model: entry.model.clone(),
            state: entry.state_label(),
        }
    }
}

#[derive(Debug, Serialize)]
struct ProjectListOutput {
    total: usize,
    projects: Vec<ProjectSnapshot>,
}

#[derive(Debug, Serialize)]
struct RestartOutput {
    projects: Vec<ProjectSnapshot>,
}

impl ProjectRuntimeFilter {
    fn matches_state(&self, state: &str) -> bool {
        let state = state.to_lowercase();
        match self {
            ProjectRuntimeFilter::Running => state == "running",
            ProjectRuntimeFilter::Stopped => state == "stopped" || state == "pending",
            ProjectRuntimeFilter::Failed => state == "failed" || state == "restarting",
        }
    }
}
