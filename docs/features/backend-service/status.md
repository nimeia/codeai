# `status` 命令详细设计

## 1. 协议层 (`protocol` crate)

### 请求 (`Request`)

- 修改 `Request` 枚举中的 `Status` 变体，使其可以携带参数：
  - 从 `Status` 改为 `Status(StatusRequest)`。
- 定义 `StatusRequest` 结构体，用以指定查询目标：
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct StatusRequest {
      // 如果为 None，则目标是 Master；否则为指定的 Worker。
      pub target: Option<ProjectRef>,
  }
  ```

### 响应 (`Response`)

- `Response::Status` 的载荷 `StatusResponse` 将被设计为一个枚举，以区分 Master 和 Worker 的状态：
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(rename_all = "snake_case")]
  pub enum StatusResponse {
      Master(MasterStatus),
      Worker(WorkerStatus),
  }
  ```
- **`MasterStatus` 结构体** (守护进程的聚合状态):
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct MasterStatus {
      pub pid: u32,
      pub uptime_secs: u64,
      // 所有 Worker 的状态摘要列表
      pub workers: Vec<WorkerSummary>,
  }
  ```
- **`WorkerSummary` 结构体** (用于 Master 的列表):
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct WorkerSummary {
      pub project_id: String,
      pub project_root: PathBuf,
      pub pid: u32,
      pub status: WorkerState, // 例如: Running, Idle, Indexing, Failed
      pub indexed_files_count: u64,
      pub uptime_secs: u64,
  }
  ```
- **`WorkerStatus` 结构体** (单个 Worker 的详细状态):
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct WorkerStatus {
      pub summary: WorkerSummary, // 复用摘要信息
      pub indexer_state: IndexerState,
      pub task_queue_size: usize,
      pub is_watching: bool, // 是否在监听文件变更
  }
  ```
- **状态枚举**:
  ```rust
  // Worker 的总体状态
  #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
  #[serde(rename_all = "snake_case")]
  pub enum WorkerState {
      Running, Idle, Indexing, Paused, Failed, Starting, Stopping,
  }

  // 索引器的详细状态
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct IndexerState {
      pub state: WorkerState, // 复用 WorkerState
      pub progress_percent: Option<f32>, // 索引进度百分比
      pub current_file: Option<String>,  // 正在处理的文件
  }
  ```

## 2. CLI (`cli` crate)

### 命令定义

- 在 `main.rs` 中增加 `Status(StatusArgs)` 子命令。
- `StatusArgs` 将是一个空结构体，因为 `project status` 命令已存在。

### 命令处理与输出

- 当执行 `code-nav status` 时:
  - CLI 向 Master 发送 `StatusRequest { target: None }`。
  - 收到 `StatusResponse::Master` 后，将其中的 `workers` 列表格式化为表格：
    ```
    ID                  PATH                STATUS      INDEXED     UPTIME
    a1b2c3d4            /path/to/proj1      Running     1052 files  1 hour
    e5f6g7h8            /path/to/proj2      Indexing    234 files   30 mins
    i9j0k1l2            /path/to/proj3      Failed      -           -
    ```
- 当执行 `code-nav project status <project>` 时:
  - CLI 向 Master 发送 `StatusRequest { target: Some(...) }`。
  - 收到 `StatusResponse::Worker` 后，格式化为详细的键值对：
    ```
    Project: a1b2c3d4 (/path/to/proj1)
    Status:  Indexing (45.2%)
    File:    src/server/main.rs
    Uptime:  1 hour
    Watcher: Active
    Queue:   12 tasks pending
    ```

## 3. 服务端 (`server` crate)

### Master (守护进程)

- **状态管理**: 在内存中维护一个所有 Worker 的状态摘要表 (`ConcurrentHashMap<ProjectId, WorkerSummary>`)。
- **心跳机制**: Worker 通过定期发送的心跳（例如，一个包含 `WorkerSummary` 的 `Request::Heartbeat`）来更新它在 Master 中的状态。
- **请求处理**: 收到 `StatusRequest` 后：
  - 如果 `target` 为 `None`，则直接使用内存中的状态表构建 `MasterStatus` 并返回。
  - 如果 `target` 指向一个项目，则将请求**转发**给对应的 Worker。

### Worker (项目工作进程)

- **请求处理**: 收到来自 Master 的 `StatusRequest` 后，从自身的各个组件（索引器、任务队列等）收集实时、详细的状态，构建 `WorkerStatus` 并返回。
- **心跳发送**: 定期构建自身的 `WorkerSummary`，并通过一个独立的 `Heartbeat` 请求发送给 Master。
