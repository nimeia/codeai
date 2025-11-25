use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "payload")]
pub enum Request {
    Info(InfoRequest),
    Status(StatusRequest),
    Search(SearchRequest),
    Goto(GotoRequest),
    List(ListRequest),
    Tree(TreeRequest),
    Index(IndexRequest),
    Logs(LogsRequest),

    // Project management
    ProjectAdd(ProjectAddRequest),
    ProjectRemove(ProjectRemoveRequest),
    ProjectList(ProjectListRequest),
    ProjectStatus(ProjectStatusRequest),
    ProjectRestart(ProjectRestartRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    Info(InfoResponse),
    Status(StatusResponse),
    Search(SearchResponse),
    Goto(GotoResponse),
    List(ListResponse),
    Tree(TreeResponse),
    Index(IndexResponse),
    Logs(LogsResponse),
    Error(ErrorBody),

    // Project management
    ProjectAdd(ProjectAddResponse),
    ProjectRemove(ProjectRemoveResponse),
    ProjectList(ProjectListResponse),
    ProjectStatus(ProjectStatusResponse),
    ProjectRestart(ProjectRestartResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub id: String,
    pub data: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InfoResponse {
    Master(MasterInfo),
    Worker(WorkerInfo),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusResponse {
    Master(MasterStatus),
    Worker(WorkerStatus),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub top_k: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub file: String,
    pub line: u32,
    pub score: f32,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GotoRequest {
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GotoResponse {
    pub file: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRequest {
    pub kind: ListKind,
    pub filter: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListResponse {
    pub items: Vec<ListItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeRequest {
    pub path: Option<String>,
    pub depth: Option<u32>,
    pub include_hidden: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeResponse {
    pub root: TreeNode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub children: Vec<TreeNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexRequest {
    pub mode: IndexMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexResponse {
    pub started: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogsRequest {
    pub target: LogTarget,
    pub since: Option<i64>,
    pub limit: Option<u32>,
    pub follow: bool,
    pub follow_interval_ms: Option<u64>,
    pub level: Option<LogLevel>,
    pub format: LogOutputFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LogsResponse {
    pub events: Vec<LogEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogTarget {
    Master,
    Worker(ProjectRef),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectRef {
    Path(String),
    Id(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogOutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    pub ts: i64,
    pub level: LogLevel,
    pub source: String,
    pub target: Option<String>,
    pub message: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    NotIndexed,
    Busy,
    InternalError,
    Unsupported,
    PermissionDenied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListKind {
    Classes,
    Methods,
    Files,
    Tree,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListItem {
    pub name: String,
    pub location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexMode {
    Full,
    Incremental,
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectAddRequest {
    pub project_root: PathBuf,
    pub id: Option<String>,
    pub autostart: bool,
    pub watch: bool,
    pub index_mode: IndexMode,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectAddResponse {
    pub project_id: String,
    pub project_root: PathBuf,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRemoveRequest {
    pub project: ProjectRef,
    pub force: bool,
    pub keep_data: bool,
    pub grace_sec: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRemoveResponse {
    pub project_id: String,
    pub project_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectListRequest {
    pub status: Option<ProjectRuntimeFilter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectListResponse {
    pub projects: Vec<ProjectInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectStatusRequest {
    pub project: ProjectRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectStatusResponse {
    pub info: ProjectInfo,
    // pub metrics: ... // TODO: Add metrics later
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRestartRequest {
    pub projects: Vec<ProjectRef>,
    pub wait: bool,
    pub force: bool,
    pub grace_sec: Option<u64>,
    pub timeout_sec: Option<u64>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRestartResponse {
    pub projects: Vec<ProjectInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInfo {
    pub project_id: String,
    pub project_root: PathBuf,
    pub autostart: bool,
    pub watch: bool,
    pub index_mode: IndexMode,
    pub model: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectRuntimeFilter {
    Running,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoRequest {
    pub target: Option<ProjectRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterInfo {
    pub protocol_version: String,
    pub server_version: String,
    pub pid: u32,
    pub uptime_secs: u64,
    pub projects_managed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInfo {
    pub protocol_version: String,
    pub server_version: String,
    pub project_id: String,
    pub project_root: PathBuf,
    pub pid: u32,
    pub uptime_secs: u64,
    pub config_summary: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusRequest {
    pub target: Option<ProjectRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterStatus {
    pub pid: u32,
    pub uptime_secs: u64,
    pub workers: Vec<WorkerSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerSummary {
    pub project_id: String,
    pub project_root: PathBuf,
    pub pid: u32,
    pub status: WorkerState,
    pub indexed_files_count: u64,
    pub uptime_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStatus {
    pub summary: WorkerSummary,
    pub indexer_state: IndexerState,
    pub task_queue_size: usize,
    pub is_watching: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Running,
    Idle,
    Indexing,
    Paused,
    Failed,
    Starting,
    Stopping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexerState {
    pub state: WorkerState,
    pub progress_percent: Option<f32>,
    pub current_file: Option<String>,
}
