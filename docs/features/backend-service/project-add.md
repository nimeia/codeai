# `code-nav project add` 任务说明

## 1. 目标与范围
- 让 `code-nav project add` 只负责“注册 + 启动流程编排”，避免 master 直接操作项目目录。
- 将项目目录初始化、`.code-nav/` 构建与索引器自举完全交给 worker/supervisor。
- 在协议、CLI、registry 与状态查询之间建立一致的数据结构，方便后续 `project status/list` 共享。

## 1.1 `project add` 触发后的编排概览
- **CLI 打包请求并返回初始状态**：`code-nav project add` 解析 `--project/--id/--autostart/--watch/--index-mode/--model` 等参数，序列化为 `ProjectAddRequest` 发送给 master；收到 `ProjectAddResponse` 后依据 `Registered|Duplicate|Invalid|Failed` 与 `QueuedWorkerState(Pending|Starting|Ready|Failed)` 渲染提示或 JSON，CLI 不直接触盘。
- **Master 三步职责**：1）使用 `RegistryWriter` 规范化路径、检查重复/嵌套并写入 `registry.json`；2）将待初始化项目写入 `runtime_dir/projects/pending/<project_id>.json`（或内存队列），立即返回 CLI；3）在状态存储中维护 `queued_worker_state/last_state/last_running/last_error` 等，后续跟随 worker 事件更新。
- **Supervisor 与 worker 启动链路**：Supervisor 监听 pending 队列，遵守 `max_concurrent_starts` 启动 worker，并注入 `project_id/root`、worker socket、`runtime_dir`、worker 配置；worker `bootstrap(project_root)` 负责创建/升级 `.code-nav/`、初始化 `metadata.db` 和向量索引、启动索引器/监听器/RPC，ready 后上报状态。
- **状态事件复用**：worker 生命周期通过 `WorkerEvent::{Started, Ready, Failed, Stopped}` 上报，master 将事件映射为 `ProjectRuntimeState`，供 `project add/status/list` 共用，保证 worker 未就绪时也能展示 `Pending/Starting` 并提示后续跟进。

## 2. CLI → Master 交互流程
1. CLI 解析参数：`--project <path>`（必填）、`--id <custom-id>`、`--autostart`、`--watch`、`--index-mode`、`--model`、`--json` 等。
2. 将参数序列化为 `ProjectAddRequest`（定义在 `crates/protocol`）：包含项目根路径、可选自定义 ID、自动策略（`autostart/watch/index_mode/model`）以及期望的输出模式。
3. 通过控制 socket 发送请求，等待 master 立即返回 `ProjectAddResponse`。
4. CLI 根据响应的 `state`（`Registered|Duplicate|Invalid|Failed`）、`queued_worker_state`（`Pending|Starting|Ready|Failed`）渲染用户提示，并在 `--json` 模式下输出结构化结果。

## 3. Master 职责拆解
1. **Registry 写入**
   - 在 `ProjectRegistry` 中拆分 `load` 与 `write`：只负责读写 `registry.json`，不再在加载阶段创建 `.code-nav/`。
   - 新增 `RegistryWriter`：处理路径规范化、重复检查、ID 分配（可根据哈希或自增 ID），并暴露 `insert/update` 方法。
2. **任务排队**
   - `project add` handler 校验路径（存在性、权限、是否嵌套其他项目）后写入 registry。
   - 将“待初始化项目”写入 `runtime_dir/projects/pending/<project_id>.json` 或内存队列，由 worker supervisor 消费。
   - 立即返回 CLI 响应，不等待 worker 完成。
3. **状态维护**
   - `registry` 或新建 `ProjectStateStore` 记录：`queued_worker_state`, `last_state`, `last_running`, `last_error`, `updated_at`。
   - Master 主循环/事件分发器将 pending 项目交给 supervisor，接收 worker 回调后更新状态；失败任务可自动重试或标记 degraded。

## 4. Worker / Supervisor 自举
1. **Supervisor 启动**
   - 新建 `crates/server/src/daemon/worker_supervisor.rs`（命名示例）：监听 pending 队列，根据 `max_concurrent_starts` 顺序启动 worker。
   - 为每个任务准备参数（`project_id`, `project_root`, `worker_socket`, `runtime_dir`, `worker_config`）。
2. **Worker 自检与初始化**
   - Worker 入口新增 `bootstrap(project_root)`：
     - 创建/升级 `project_root/.code-nav/`，准备 `metadata.db`、向量索引、配置文件。
     - 校验锁/权限，必要时修复或报错。
     - 启动索引器、watcher、RPC 监听，确认 ready 后向 master 报告。
3. **状态上报**
   - Worker 在生命周期关键点上报 `WorkerEvent::{Started, Ready, Failed{reason}, Stopped}`。
   - Master 依据事件更新 `queued_worker_state`，供 CLI 查询；失败信息写入 `last_error` 并可触发重试。
4. **启动前检查项（防重复、防脏数据）**
   - Registry 中是否已有同路径项目且状态为 `Ready/Starting`，避免重复启动。
   - 运行时目录 `runtime_dir/projects/<id>/` 下是否存在活跃 PID 文件、socket/pipe 是否可连接；若进程仍活着则直接复用。
   - `.code-nav/lock` 或内部锁是否由存活进程持有，防止双重索引；若为僵尸锁需清理后再启动。
   - `.code-nav/metadata.db`、向量索引等核心文件是否完整且读写可用，必要时做修复或降级为重建。
   - 上次启动失败的错误/崩溃标记是否存在，是否需要走冷启动（重建索引）或限制重试次数。

## 5. 协议结构
- `ProjectAddRequest { project_root: PathBuf, project_id: Option<String>, autostart: bool, watch: bool, index_mode: Option<IndexMode>, model: Option<String>, format: OutputFormat }`
- `ProjectAddResponse { state: AddState, project_id: String, project_root: PathBuf, queued_worker_state: WorkerInitState, message: String }`
- `AddState = Registered | Duplicate | Invalid | Failed`；`WorkerInitState = Pending | Starting | Ready | Failed { reason }`。
- Worker 事件通过 `MasterEvent::WorkerStateChanged { project_id, state, error }` 回传，master 同步更新 registry/state store。

## 6. CLI 输出规范
- 默认（非 JSON）：
  ```
  Project registered: <project_id>
  Worker: initializing (pending)
  Hint: run `code-nav project status --project <path>` to follow progress.
  ```
- JSON 模式：输出 `{ "state": "Registered", "project_id": ..., "worker": { "state": "Pending" } }`，供脚本解析。
- 错误场景：
  - 路径重复：`state=Duplicate`，message 提供现有 ID。
  - 权限/路径无效：`state=Invalid`，提示原因。
  - Registry 写入失败：`state=Failed`，CLI 退出码非零。

## 7. 状态查询复用
- `project add` 响应与 `project status/list` 使用统一 `ProjectRuntimeState` 结构：`{ project_id, project_root, state, worker { pid, socket, init_state, last_heartbeat }, indexing { mode, last_indexed_at, pending_tasks } }`。
- `project status` 可在 worker 未 ready 时展示“Pending/Starting”，并附带最近事件时间戳。
- `project list` 在表格模式增加 `Init` 列，显示 `pending/starting/ready/failed`。

## 8. 验收清单
1. CLI：参数解析、help/usage、JSON/人类输出、退出码、重复路径提示。
2. Protocol：`ProjectAddRequest/Response` 序列化测试，`AddState/WorkerInitState` 枚举确保向后兼容。
3. Master：registry 写入单元测试（重复、嵌套路径、ID 分配）、pending 队列写入、事件驱动状态更新。
4. Worker：`bootstrap` 单元/集成测试（已有 `.code-nav/`、首次创建、权限错误）。
5. E2E：运行 master → 执行 `project add`，观察 CLI 提示、registry 更新、worker 启动、`project status` 状态变化。
