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
2. 默认 `--target master`；当目标为 `worker` 时必须提供 `--project <path|id>`，CLI 负责解析并在请求中带上 `ProjectRef`。当前 master 端仅实现 **master** 历史回放，收到 `LogTarget::Worker` 将返回 `Unsupported` 错误。
3. `--limit` 控制历史回放数量（默认 500，最大 10k）；`--level` 用于 server 端过滤日志级别。
4. 输出模式：文本时按 `[YYYY-MM-DD HH:MM:SS][LEVEL][source] message` 格式；`--json` 时逐行输出 `LogEvent` JSON。

## 3. 时间/输出控制规则
- `--since` 可输入相对时长（`10m`、`2h`、`1d`）或 RFC3339（`2024-04-18T10:00:00Z`）。CLI 将其转换为 Unix 时间戳（秒）传入 `LogsRequest`。
- 若同时指定 `--since` 与 `--limit`，server 先按时间过滤，再截断到 limit。
- `--follow/-f`：当前 master 端尚未提供流式 follow；CLI 请求 follow 会收到 `Unsupported` 错误，后续实现时遵循“先历史后实时”的规则。
- 文本模式颜色遵循 `--color`：`auto`（仅 TTY）、`always`、`never`。不同 level 上色规则需在 CLI formatter 中统一实现。

## 4. Master 端日志聚合服务
1. **LogsService 组件**
   - 驻留于 `crates/server/src/master/logs.rs`，通过 `VecDeque<LogEvent>` ring buffer 管理近期历史。容量默认 10k，可在配置中调优；溢出时淘汰最旧事件。
   - 持久落盘：master 常规日志输出仍由 tracing subscriber 负责；LogsService 专注于内存回放，不再直接读取文件补齐历史，后续可以在此基础上扩展文件回溯。
2. **tracing 捕获层**
   - 使用 `CaptureLayer` 作为 tracing layer，覆盖 master 当前 subscriber，将事件转换为 `LogEvent` 并写入 `LogsService`。
   - 捕获层会刷新 `ts` 字段为当前 Unix 时间戳秒值，沿用原始 `target`、`message` 与结构化字段。
3. **RPC 接口**
   - `LogsHistory(LogsRequest) -> LogsResponse { events: Vec<LogEvent> }`：一次性回放 master 历史。
   - `LogsStream(LogsRequest)` 暂未开放；收到 follow 请求会返回 `Unsupported` 错误，避免误导客户端。
   - Server 端在收集时处理 `limit/since/level` 过滤，确保不会回传多余数据。

## 5. Worker 侧日志上报
1. 设计目标保持不变：统一使用 `tracing` subscriber 输出到本地和 supervisor 通道；`WorkerLogEvent` 序列化为 `LogEvent`，`source="worker:<project_id>"`，`target` 记录组件名。
2. 当前阶段 master 端尚未接入 worker 流，也不会持有 worker 专用缓冲。收到 `LogTarget::Worker` 请求将直接返回 `Unsupported`，避免误报 worker 状态。
3. 后续接入时需补齐：worker 停止事件广播、按项目维度的 `broadcast` 订阅、以及离线错误码处理。

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
      pub ts: i64,                        // Unix 时间戳（秒）
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

## 8. 存储格式
- 落盘/暂存/传输统一采用 NDJSON 的 `LogEvent` 结构，详见 @/docs/features/backend-service/log-storage-format.md。
- master 写文件时使用秒级 Unix 时间戳且不带 ANSI；worker spool 使用相同格式以便断点续传与批量推送。

## 9. 当前阶段实现状态
- master 侧：`LogsService` 已接入 tracing 捕获层，支持 master 历史回放、级别与时间过滤、缓冲容量配置；follow 流与文件补齐尚未实现。
- worker 侧：请求会收到 `Unsupported`，待后续接入 worker 日志上报与 spool 传输后再开放。

## 10. 验收清单
1. **CLI**：参数解析、`--help`、颜色控制、文本 vs JSON 输出、follow 模式中断/退出码单测。
2. **Protocol**：`LogsRequest/LogEvent` 序列化测试，确保 `LogLevel` 与 `ProjectRef` 兼容已有协议。
3. **Master LogsService**：
   - Ring buffer 入队/出队测试。
   - `--since` + limit 组合过滤测试。
   - 并发 follow 订阅测试（master/多个 worker）。
4. **Worker 日志上报**：模拟 worker 发送日志/停止事件，确保 master 能关闭 follow 流并提示 `worker stopped`。
5. **E2E**：启动 master + worker，执行 `code-nav logs` 的 master/worker/JSON/follow 场景，验证历史回放、实时流与错误码。
