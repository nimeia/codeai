# `info` 命令详细设计

## 1. 协议层 (`protocol` crate)

### 请求 (`Request`)

- 修改 `Request` 枚举中的 `Info` 变体，使其可以携带参数：
  - 从 `Info` 改为 `Info(InfoRequest)`。
- 定义 `InfoRequest` 结构体，用以指定查询目标：
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct InfoRequest {
      // 如果为 None，则目标是 Master；否则为指定的 Worker。
      pub target: Option<ProjectRef>,
  }
  ```

### 响应 (`Response`)

- `Response::Info` 的载荷 `InfoResponse` 将被设计为一个枚举，以区分 Master 和 Worker 的信息：
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(rename_all = "snake_case")]
  pub enum InfoResponse {
      Master(MasterInfo),
      Worker(WorkerInfo),
  }
  ```
- **`MasterInfo` 结构体** (守护进程信息):
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct MasterInfo {
      pub protocol_version: String, // 协议版本
      pub server_version: String,   // 服务版本
      pub pid: u32,                 // 进程 ID
      pub uptime_secs: u64,         // 运行时长
      pub projects_managed: usize,  // 管理的项目总数
  }
  ```
- **`WorkerInfo` 结构体** (项目工作进程信息):
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize)]
  pub struct WorkerInfo {
      pub protocol_version: String,
      pub server_version: String,
      pub project_id: String,       // 项目 ID
      pub project_root: PathBuf,    // 项目根目录
      pub pid: u32,
      pub uptime_secs: u64,
      // 配置摘要，例如：{"index_mode": "auto", "watch": "true"}
      pub config_summary: BTreeMap<String, String>,
  }
  ```

## 2. CLI (`cli` crate)

### 命令定义

- 在 `main.rs` 中增加 `Info(InfoArgs)` 子命令。
- `InfoArgs` 结构体将包含一个可选参数，用于指定项目：
  ```rust
  #[derive(Debug, Args)]
  pub struct InfoArgs {
      #[arg(long = "project", value_name = "PATH|ID")]
      pub project: Option<String>,
  }
  ```

### 命令处理与输出

- 当执行 `code-nav info` 时 (无 `--project`):
  - CLI 向 Master 发送 `InfoRequest { target: None }`。
  - 收到 `InfoResponse::Master` 后，格式化输出：
    ```
    CodeNav Master
    Version:        0.1.0
    Protocol:       1.0
    PID:            12345
    Uptime:         2 hours 34 minutes
    Projects:       5
    ```
- 当执行 `code-nav info --project <...>` 时:
  - CLI 向 Master 发送 `InfoRequest { target: Some(...) }`。
  - 收到 `InfoResponse::Worker` 后，格式化输出：
    ```
    CodeNav Worker
    Project ID:     a1b2c3d4
    Project Root:   /path/to/project
    Version:        0.1.0
    PID:            54321
    Uptime:         1 hour 2 minutes
    Config:
      - index_mode: auto
      - watch:      true
    ```

## 3. 服务端 (`server` crate)

### Master (守护进程)

- **请求处理**: 收到 `InfoRequest` 后：
  - 如果 `target` 为 `None`，则收集自身的 PID、版本、运行时长等信息，构建 `MasterInfo` 并返回。
  - 如果 `target` 指向一个项目，则从项目注册表中找到对应的 Worker，并将请求**转发**给它，然后将 Worker 的响应再**原样返回**给 CLI。

### Worker (项目工作进程)

- **请求处理**: 收到来自 Master 的 `InfoRequest` 后，收集自身的项目信息、PID、配置摘要等，构建 `WorkerInfo` 并返回给 Master。
