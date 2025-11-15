# `code-nav project list` 任务说明

## 1. 目标与范围
- 输出 master registry 中所有已注册项目的统一视图：包含 `project_id`、项目根路径、autostart/watch 策略、当前 worker/runtime 状态、索引/诊断摘要。
- 允许用户通过 `--status/--autostart-only/--watch-only/--limit/--offset` 等选项筛选结果，支持 `--json` 与默认表格两种输出模式。
- 与 `project add/remove/status` 共享状态结构，确保 CLI、协议、master、worker 对项目状态的一致解释，方便脚本与人类使用。

## 2. CLI → Master 交互流程
1. CLI 解析参数：`--format table|json`（默认 table）、`--status <state>`、`--autostart-only`、`--watch-only`、`--limit <n>`、`--offset <n>`、`--sort-by <field>`、`--verbose` 等。
2. CLI 构造 `ProjectListRequest`（定义在 `crates/protocol`），字段包含过滤条件（状态、策略）、分页参数（limit/offset）、排序字段，以及输出模式。
3. 通过控制 socket 发送请求，master 立即返回 `ProjectListResponse`，内容包含匹配项目的数组、总数量、分页信息。
4. CLI 根据 `format` 渲染：
   - `table` 模式显示对齐列：`ID | Path | Autostart | Watch | State | Worker PID | Last Seen | Index Rev`，必要时换行展示错误信息；
   - `json` 模式输出结构化数组，保留所有字段供脚本消费。

## 3. Master 数据来源
1. **Registry 快照**：`ProjectRegistry` 暴露 `iter_entries()` 或 `list(filter)` API，一次性读取 `registry.json` 或内存副本，包含静态字段（路径、策略、创建时间）。
2. **Runtime 状态**：`ProjectStateStore`（或等价结构）维护 `ProjectRuntimeState { init_state, runtime_state, worker_pid, worker_socket, last_seen, index_revision, pending_jobs, last_error }`，由 worker/supervisor 事件驱动更新。
3. **聚合逻辑**：`project list` handler 组合 registry 与 state store，生成 `ProjectListItem`：
   - 当某项目尚未有 worker（比如刚 add）时，`runtime_state` 显示 `Pending` 并附带 `queued_worker_state`；
   - 当 worker 已被移除但数据保留时，`state` 显示 `Removed` 或 `Inactive` 并提供提示；
   - 支持 `Removing`、`Restarting` 等过渡态，供 CLI 展示。

## 4. Worker / Supervisor 状态上报要求
1. Worker 必须在生命周期事件（`Started`, `Ready`, `Heartbeat`, `Stopping`, `Stopped`, `Failed { reason }`）中上报 `index_revision`, `pending_jobs`, `last_error`, `watcher.enabled` 等指标。
2. Supervisor 将事件转译为 `ProjectRuntimeState` 更新，记录 `last_seen` 时间戳与 `worker_pid`/`socket`。
3. 若 worker 心跳超时，master 将状态降级为 `Unreachable`，并在 list 输出 `state=Failed` + `message="heartbeat timeout"`。

## 5. 协议结构
- `ProjectListRequest { format: OutputFormat, status: Option<ProjectStatusFilter>, autostart_only: bool, watch_only: bool, limit: Option<u32>, offset: u32, sort_by: ProjectListSort, verbose: bool }`
- `ProjectStatusFilter = Pending | Starting | Ready | Failed | Removing | Inactive | Any`；CLI 默认 `Any`。
- `ProjectListSort = ByName | ByState | ByLastSeen | ByCreatedAt`；默认 `ByName`。
- `ProjectListResponse { total: u32, returned: u32, items: Vec<ProjectListItem> }`
- `ProjectListItem { project_id: String, project_root: PathBuf, autostart: bool, watch: bool, created_at: SystemTime, runtime_state: RuntimeStateSummary, worker: Option<WorkerSummary>, indexing: Option<IndexingSummary>, last_error: Option<String> }`
- `RuntimeStateSummary { init_state: WorkerInitState, state: WorkerRuntimeState, updated_at: SystemTime }`
- `WorkerSummary { pid: Option<u32>, socket: Option<PathBuf>, last_seen: SystemTime }`
- `IndexingSummary { revision: Option<String>, pending_jobs: u32, last_indexed_at: Option<SystemTime> }`

## 6. CLI 输出与过滤
1. **表格模式**：使用 `tabled`/`comfy-table`（或现有 formatter）渲染列：
   - 基础列：`ID`, `Path`, `State`, `Autostart`, `Watch`, `Worker`, `Last Seen`；
   - `--verbose` 时追加 `Index Rev`, `Pending`, `Last Error`；若错误较长则折行或限制长度。
2. **JSON 模式**：输出 `ProjectListResponse` 的原始结构，必要时再额外添加 `meta: { total, returned, limit, offset }`。
3. **过滤行为**：
   - CLI 可选择在本地过滤或把过滤条件交给 master（推荐后者以减少数据传输）；
   - `--status` 多次提供时取最后一次或报错；`--status any` 为默认；
   - `--autostart-only/--watch-only` 彼此可叠加，等价于 `autostart=true && watch=true`。
4. **分页提示**：若 `total > returned + offset`，在表格底部输出“显示 X-Y / 共 N，使用 --offset/--limit 查看更多”。

## 7. 性能与一致性
1. `list` 请求应在 O(n) 内完成，其中 n 为匹配项目数；registry/state 读取需避免持久锁，使用读锁或快照。
2. 允许 master 缓存上一次 list 结果的序列化字符串，当 registry/state 未变化且请求条件一致时可复用（可选优化）。
3. CLI 需确保输出稳定排序（默认按 `project_id` 或 `normalized path`），方便脚本 diff；JSON 模式使用结构化字段避免顺序依赖。

## 8. 验收清单
1. CLI：参数解析测试（含冲突/重复）、帮助文档、表格与 JSON 输出对齐、过滤与分页在单元或集成测试中覆盖。
2. Protocol：`ProjectListRequest/Response` 序列化测试、`ProjectListItem` 字段兼容性、`ProjectListSort`/`ProjectStatusFilter` 枚举保持向后兼容。
3. Master：registry + state store 聚合单元测试、过滤/排序/分页逻辑覆盖、心跳超时导致的状态降级模拟。
4. Worker/Supervisor：事件上报包含 index 版本/错误等诊断字段的测试，确保 list 输出在各种状态（Pending/Ready/Failed/Removing/Inactive）下均有合理数据。
5. 集成：启动 master + 多个项目（含 autostart/watch/不同状态），执行 `code-nav project list` 的 table/json/过滤/分页路径，验证 CLI 输出与 registry/state 一致。
