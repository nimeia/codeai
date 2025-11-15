# `code-nav logs` 任务说明

## 1. 目标与范围
- 为 master 与各项目 worker 提供统一的日志查看命令，支持历史回放与实时跟随。
- CLI → master → worker：CLI 解析参数，发送 `LogsRequest`，master 负责聚合并按需回放/流式转发。
- 兼顾人类阅读（彩色文本、limit 控制）与脚本消费（JSON 事件）。

## 2. CLI 参数与命令结构
1. 命令格式：
   ```
   code-nav logs \
     [--target <master|worker>] \
     [--project <path|id>] \
     [--since <duration|RFC3339>] \
     [--limit <n>] \
     [--follow|-f] \
     [--follow-interval <ms>] \
     [--level <trace|debug|info|warn|error>] \
     [--json] [--color auto|always|never]
   ```
2. 默认 `--target master`；当目标为 `worker` 时必须提供 `--project <path|id>`，CLI 负责解析并在请求中带上 `ProjectRef`。
3. `--limit` 控制历史回放数量（默认 500，最大 10k）；`--level` 用于 server 端过滤日志级别。
4. 输出模式：文本时按 `[YYYY-MM-DD HH:MM:SS][LEVEL][source] message` 格式；`--json` 时逐行输出 `LogEvent` JSON。

## 3. 时间/输出控制规则
- `--since` 可输入相对时长（`10m`、`2h`、`1d`）或 RFC3339（`2024-04-18T10:00:00Z`）。CLI 将其转换为 Unix 时间戳传入 `LogsRequest`。
- 若同时指定 `--since` 与 `--limit`，server 先按时间过滤，再截断到 limit。
- `--follow/-f`：CLI 先打印历史记录，再保持长连接；`--follow-interval` 决定 CLI 刷新频率（默认 250ms）。
- 文本模式颜色遵循 `--color`：`auto`（仅 TTY）、`always`、`never`。不同 level 上色规则需在 CLI formatter 中统一实现。

## 4. Master 端日志聚合服务
1. **LogsService 组件**
   - 驻留于 `crates/server/src/master/logs.rs`（建议新文件），负责管理历史缓冲、文件回溯与 worker 流。
   - 历史缓冲：基于 `VecDeque<LogEvent>` 的 ring buffer，容量默认 10k，可在配置中调优；溢出时淘汰最旧事件。
   - 持久落盘：可选写入 `runtime_dir/logs/master.log`，当 `--since` 早于内存缓冲时，从文件尾部回溯补齐。
2. **Worker 日志流管理**
   - Supervisor 捕获 worker 的 stdout/stderr 或 `tracing` sink，通过 IPC 将 `LogEvent` 转发给 master。
   - LogsService 为每个 `project_id` 维护一个 `broadcast::Sender<LogEvent>`；`worker` 目标订阅该 sender，支持多客户端并发。
3. **RPC 接口**
   - `LogsHistory(LogsRequest) -> LogsResponse { events: Vec<LogEvent> }`：一次性回放。
   - `LogsStream(LogsRequest) -> stream<LogEvent>`：若 `follow=true` 则保持连接；若 `follow=false`，stream 在发送完历史记录后立即关闭。
   - Server 负责处理 `limit/since/level/project` 等过滤，确保不会回传多余数据。

## 5. Worker 侧日志上报
1. Worker 使用统一的 `tracing` subscriber，将事件输出到：
   - 本地文件/STDOUT（供调试）。
   - Supervisor 通道（`WorkerLogEvent`），由 master 聚合。
2. `WorkerLogEvent` 序列化为 `LogEvent`，字段中 `source="worker:<project_id>"`，`target` 记录具体组件（`watcher`, `indexer` 等）。
3. 当 worker 停止时，supervisor 需向 master 发送 `LogEvent`（level=`INFO`，message=`worker stopped`）并关闭对应的 `broadcast`。
4. 若 worker 离线而 CLI 请求 `--follow`，LogsService 应立即返回 `WorkerOffline` 错误或提供仅限历史缓冲的数据，并提示用户 worker 已停止。

## 6. 协议与数据结构
- 在 `crates/protocol/src/logs.rs` 定义：
  ```rust
  pub struct LogsRequest {
      pub target: LogTarget,              // Master | Worker(ProjectRef)
      pub since: Option<Timestamp>,
      pub limit: Option<u32>,
      pub follow: bool,
      pub level: Option<LogLevel>,
      pub format: OutputFormat,           // Table | Json
  }

  pub enum LogTarget {
      Master,
      Worker(ProjectRef),
  }

  pub struct LogEvent {
      pub ts: Timestamp,
      pub level: LogLevel,
      pub source: String,                 // master | worker:<id>
      pub target: Option<String>,         // 组件名，如 watcher/indexer
      pub message: String,
      pub fields: BTreeMap<String, String>,
  }
  ```
- `LogsResponse` 在非 follow 模式下包含 `events: Vec<LogEvent>`；follow 模式通过 streaming/RPC channel 传输。
- 错误响应结构：`LogsError { code: LogsErrorCode, message: String }`，CLI JSON 模式需原样输出。

## 7. 错误处理与退出码
- CLI 退出码：`0`（成功）、`2`（项目不存在或 worker 未运行）、`3`（follow 超时/被中断）、`1`（其他错误）。
- Server 错误枚举：
  - `ProjectNotFound`：`--project` 未在 registry 中注册。
  - `WorkerOffline`：项目存在但 worker 不在运行；history 可返回，follow 则立即结束。
  - `InvalidSince`：`--since` 解析失败或超出允许范围。
  - `FollowTimeout`：长连接在 `idle_timeout` 内未收到事件，server 发送一条 `WARN` 级别 `LogEvent` 后关闭。
  - `PermissionDenied`：为未来多用户/ACL 预留。
- CLI 文本模式遇到错误时输出 `Error (<code>): <message>`；JSON 模式输出 `{ "error": { "code": "ProjectNotFound", "message": "..." } }`。

## 8. 验收清单
1. **CLI**：参数解析、`--help`、颜色控制、文本 vs JSON 输出、follow 模式中断/退出码单测。
2. **Protocol**：`LogsRequest/LogEvent` 序列化测试，确保 `LogLevel` 与 `ProjectRef` 兼容已有协议。
3. **Master LogsService**：
   - Ring buffer 入队/出队测试。
   - `--since` + limit 组合过滤测试。
   - 并发 follow 订阅测试（master/多个 worker）。
4. **Worker 日志上报**：模拟 worker 发送日志/停止事件，确保 master 能关闭 follow 流并提示 `worker stopped`。
5. **E2E**：启动 master + worker，执行 `code-nav logs` 的 master/worker/JSON/follow 场景，验证历史回放、实时流与错误码。
