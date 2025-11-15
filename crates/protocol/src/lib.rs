use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "payload")]
pub enum Request {
    Info,
    Status,
    Search(SearchRequest),
    Goto(GotoRequest),
    List(ListRequest),
    Index(IndexRequest),
    Logs(LogsRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    Info(InfoResponse),
    Status(StatusResponse),
    Search(SearchResponse),
    Goto(GotoResponse),
    List(ListResponse),
    Index(IndexResponse),
    Logs(LogsResponse),
    Error(ErrorBody),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub id: String,
    pub data: T,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InfoResponse {
    pub protocol_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StatusResponse {
    pub ready: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexMode {
    Full,
    Incremental,
}
