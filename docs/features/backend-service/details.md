# 后端服务功能 + CLI 设计细则

## 1. 总览任务
1. Master 启动流程（`code-navd start`）：定义配置结构、控制端点创建、前台/后台模式、日志初始化。
2. 项目注册与 worker 管理（`project add/remove/list/status/restart/stop`）：设计 registry、worker 启停、自动重启/回收策略。
3. IPC 协议扩展：在 protocol crate 中增加 master↔worker、CLI↔master 的请求/响应类型。
4. 索引/搜索 API 适配 master：明确 CLI `search/list/goto/tree/index/status/info` 的 `--project` 参数和路由逻辑。
5. Worker 生命周期细节：整理项目级启动/停止步骤（配置校验、锁文件、资源加载、watcher、信号处理）。
6. 日志与监控方案：规划 master/worker 日志级别、输出目录、轮转与 metrics/健康检查。
7. 配置与模型管理：划分全局 vs 项目级配置、定义 CLI `config/models` 的作用域与热更新流程。
8. 故障与恢复流程：制定 master/worker 异常、僵尸锁、IPC 断开等场景的检测与恢复策略。

## 2. Master 服务（code-navd）
- 架构与命令详见 @/docs/features/backend-service/master-worker.md。
- Master 核心职责：
  - 统一控制端点：创建 `master.sock`（UDS）或 `code-nav-master`（Named Pipe），供 CLI 单点访问。
  - 项目 registry：记录 `project_root`、worker PID、IPC 地址、索引状态、最近活动时间。
  - 请求路由：接收 CLI 命令，按 `project` 参数启动/选择 worker，转发 `search/goto/list/tree/index` 等 RPC 并返回结果。
  - 监控与调度：监听 worker 存活、崩溃、闲置；按策略回收或重启。
- 优雅停机：master 接受 `code-nav stop` 后发出 `Shutdown` 给所有 worker，等待其完成后再关闭自身。

### 2.1 `code-navd start` 详细流程
1. **配置解析**：流程详见 @/docs/features/backend-service/start-master-config.md；读取默认配置、环境变量与 CLI 参数，生成 `MasterConfig` 并校验控制端点与 `.code-nav/` 目录。
2. **单实例检测**：详见 @/docs/features/backend-service/start-master-single-instance.md；通过 `master.lock/master.pid` 防止重复实例，检测僵尸并输出提示。
3. **日志与监控初始化**：详见 @/docs/features/backend-service/start-master-logging.md；根据配置设置 tracing（级别、文件/STDOUT、轮转）并可选启动 metrics/health 服务。
4. **控制端点建立**：详见 @/docs/features/backend-service/start-master-endpoint.md；创建 UDS/Named Pipe/TCP 监听并完成握手与安全校验。
5. **项目 registry 恢复**：详见 @/docs/features/backend-service/start-master-registry.md；加载 registry.json，校验项目状态、清理残留并准备自动启动列表。
6. **自动启动策略**：详见 @/docs/features/backend-service/start-master-autostart.md；根据配置构建启动队列、控制并发、重试失败并可选空闲回收。
7. **主循环与信号处理**：详见 @/docs/features/backend-service/start-master-mainloop.md；注册信号、处理 CLI 命令/worker 心跳、执行自动启动与空闲回收、实现优雅停机。
8. **启动反馈**：详见 @/docs/features/backend-service/start-master-feedback.md；提供前台/后台提示、wait-ready、失败清理与系统集成钩子。

### 2.2 `code-navd stop` 详细流程
1. **命令入口**：详见 @/docs/features/backend-service/stop-master-cli.md；定义 `code-nav stop` 子命令语法、`--grace/--timeout/--force/--yes` 参数、幂等策略与 CLI 输出/退出码。
2. **协议与路由**：详见 @/docs/features/backend-service/stop-master-protocol.md；扩展 protocol crate 的 `Request::Stop`、`Response::Stop`、`StopState`，并描述权限校验与错误码（Busy/PermissionDenied 等）。
3. **守护进程关机**：详见 @/docs/features/backend-service/stop-master-shutdown.md；master 进入 `Stopping` 状态后停止监听新请求，广播 worker `Shutdown`，等待宽限期并在需要时强制终止，清理锁/Socket/PID、flush 日志与 metrics。
4. **可观测性与测试**：stop 流程需输出 `shutdown_request/shutdown_progress/shutdown_complete` 日志、metrics（`shutdown_total` 等），并覆盖幂等、超时、force、拒绝停机等路径的单元与集成测试。

### 2.3 `code-navd restart` 详细流程
1. **命令定位**：restart = stop + start，既可作用于 master（全局重启）也可作用于单个项目 worker，用于加载配置或恢复异常。
2. **CLI 设计**：`code-nav restart [--wait] [--grace <sec>] [--force]`、`code-nav project restart --project <path|id>`；提供 `--wait`（默认 true）、`--grace`（沿用 stop 宽限期）、`--force`（强制终止）、阶段提示与退出码（0 成功、2 拒绝/不存在、3 超时、1 其他）。
3. **协议扩展**：在 protocol crate 中定义 `Request::Restart(RestartRequest { scope, force, grace_secs, wait_ready })`、`Response::Restart(RestartResponse { state, message })`；`RestartScope=Master|Project(ProjectId)`、`RestartState=Accepted|AlreadyRestarting|Completed|Failed`；错误码沿用 Busy/NotIndexed/PermissionDenied/InternalError。
4. **Server 流程**：
   - Master 重启：状态切换至 `Restarting`，向所有 worker 发送 `Shutdown`，等待退出后清理锁/PID，重新启动或通知外层自动拉起；CLI 首先收到 `Accepted`，待监听器恢复后可通过 `wait_ready` 确认 `Completed`。
   - Worker 重启：主循环暂停该项目请求，发送 `WorkerShutdown { grace }`，等待退出后复用 autostart 启动流程，更新 registry 并通知结果；支持按队列逐个或限制并发。
5. **状态协作**：`state::AppState` 记录 `Restarting` 标记、最近一次重启原因与时间；registry 记录项目级重启结果；watcher/indexer 在停机前 checkpoint，重启后恢复。
6. **测试与验收**：CLI 参数解析与提示、协议序列化、server 状态机单测，以及集成测试（重启 master、单 worker、并发、多任务期间重启）覆盖成功/超时/force/拒绝流程。

## 3. 项目 worker
- 每个项目由 master 自动派生 worker 进程承担索引/搜索/监听逻辑，入口命令仍为 `code-navd`（内部子命令区分角色）。
- Worker 启动细则：
  1. 配置解析与环境校验：参见 @/docs/features/backend-service/start-config.md。
  2. 单实例保障：项目级 PID/lock 文件，防止重复 worker。
  3. 日志与监控：初始化 tracing/log（文件+STDOUT），可选 metrics。
  4. 资源加载：打开 SQLite metadata、加载 HNSW/向量索引、预热 embedding/远程客户端，初始化 `AppState`。
  5. 服务/任务启动：启动与 master 的 IPC 监听，注册 RPC 路由；启动文件 watcher 与索引调度器。
  6. 索引状态：启动时执行健康检查或触发增量索引，记录索引版本/队列。
  7. 生命周期管理：注册信号处理，确保 `project remove`/`stop` 时优雅退出；向 master 汇报状态。
- Worker 停止流程参照 @/docs/features/backend-service/stop-master-shutdown.md（由 master 通过 IPC 触发优雅退出，必要时执行强制终止）。

## 4. 索引管理
- API：`/index/full`、`/index/incremental`。
- CLI：`code-nav index full --project <path> [--watch]`、`code-nav index incremental --project <path> [--paths foo.rs]`（CLI → master → worker）。
- 行为：调度 indexer 任务，写入 metadata.db 与向量索引，watcher 监听变更并触发增量任务。

## 5. 语义搜索
- API：`/search`。
- CLI：`code-nav search "<query>" --project <path> --top-k 5 --lang rust --file-prefix src/`（CLI → master → worker）。
- 行为：文本 → embedding → HNSW 检索 → 结果排序 → 返回 `file/line/score/snippet`。

## 6. Goto 导航
- API：`/goto`。
- CLI：`code-nav goto "init http server" --project <path> [--open <editor>]`。
- 行为：语义匹配符号，返回唯一位置及上下文，可触发编辑器打开。

## 7. 结构化列表
- API：`/list/classes`、`/list/methods`、`/list/files`。
- CLI：`code-nav list classes --project <path> --filter Controller` / `list methods --project <path> --limit 50` / `list files --project <path> --lang ts`。
- 行为：基于 SQLite metadata 进行结构化查询，支持过滤、排序、分页。

## 8. 目录树
- API：`/tree`。
- CLI：`code-nav tree --project <path> [path] --depth 3 --include-hidden`。
- 行为：返回目录树与文件类型信息，供 CLI 渲染。

## 9. 运行状态与信息 (Runtime Status and Information)

详细设计: 
- @/docs/features/backend-service/info.md
- @/docs/features/backend-service/status.md

**1. 协议层设计 (`protocol` crate)**
*   **1.1. 基础信息 (`Info`)**
    *   **请求**: `Request::Info` (可携带可选的 `project_ref` 来定位目标)。
    *   **响应**: `Response::Info(InfoResponse)`，包含：
        *   `protocol_version`: 协议版本。
        *   `server_version`: 服务端版本。
        *   `role`: 角色 ("master" 或 "worker")。
        *   `project_id` / `project_root`: (仅 Worker) 项目信息。
        *   `config_summary`: (可选) 关键配置摘要。

*   **1.2. 动态状态 (`Status`)**
    *   **请求**: `Request::Status` (可携带可选的 `project_ref` 来定位目标)。
    *   **响应**: `Response::Status(StatusResponse)`，包含一个枚举：
        *   `Master(MasterStatus)`:
            *   `pid`: Master 进程 ID。
            *   `uptime_secs`: 运行时长。
            *   `worker_summary`: 所有 Worker 的状态摘要列表 (`Vec<WorkerSummary>`)。
        *   `Worker(WorkerStatus)`:
            *   `pid`: Worker 进程 ID。
            *   `uptime_secs`: 运行时长。
            *   `indexer_state`: 索引器状态 (如 "idle", "indexing", "paused")。
            *   `indexed_files_count`: 已索引文件数。
            *   `task_queue_size`: 待处理任务数。

**2. CLI 设计 (`cli` crate)**
*   **2.1. `code-nav info`**
    *   **功能**: 获取 Master 守护进程的基础信息。
    *   **输出**: 简洁的键值对，如版本、协议、运行时长等。

*   **2.2. `code-nav status`**
    *   **功能**: 以列表形式展示 Master 和所有 Worker 的摘要状态，类似 `docker ps`。
    *   **输出**: 表格，包含 `ID`, `Path`, `Status`, `Indexed Files`, `Uptime` 等列。

*   **2.3. `code-nav project status <project>`**
    *   **功能**: 获取单个项目的详细运行状态。
    *   **输出**: 详细的键值对列表，包含索引进度、任务队列、文件监听状态等。

**3. Master 守护进程设计 (`server` crate)**
*   **3.1. 状态聚合**:
    *   在内存中维护所有 Worker 的最新状态摘要。
    *   通过心跳机制或定期轮询从 Worker 处更新这些信息。
*   **3.2. 请求路由**:
    *   收到无目标的 `status` 或 `info` 请求时，返回自身和所有 Worker 的聚合信息。
    *   收到带项目目标的请求时，将其准确转发给对应的 Worker 进程处理。

**4. Worker 进程设计 (`server` crate)**
*   **4.1. 状态收集**:
    *   在自身内存中实时维护索引器、任务队列等模块的详细状态。
*   **4.2. 状态上报与响应**:
    *   响应 Master 的状态查询请求，返回详细的自身状态。
    *   (可选) 主动向 Master 发送心跳，报告健康状况和关键指标。

## 10. 配置 & 模型管理（扩展）
- CLI：`code-nav config set <key> <value>`、`code-nav config get <key>`、`code-nav models list`、`code-nav models select <name>`（可带 `--project` 针对特定 worker，或不带针对 master/global）。
- 行为：集中管理 embedding/向量后端与网络/API Key，必要时通知 master/worker 热更新。

## 11. `code-nav project` 子命令概览
- **命令结构**：`code-nav project <subcommand> [options]`，统一继承全局选项（`--socket`, `--runtime-dir`, `--json` 等）；当仅输入 `code-nav project` 时输出子命令列表与示例。
- **project add**：参数 `--project <path>`、`--id <custom-id>`、`--autostart`、`--watch`、`--index-mode`、`--model`；行为是注册项目、创建 `.code-nav/`、更新 registry 并按策略启动 worker；对应 `ProjectAddRequest`。详细拆解见 @/docs/features/backend-service/project-add.md。
- **project remove**：参数 `--project`, `--force`, `--keep-data`, `--grace`；行为是优雅停止 worker、删除锁/PID、清理 registry，可选择保留数据；对应 `ProjectRemoveRequest`。详细拆解见 @/docs/features/backend-service/project-remove.md。
- **project list**：参数 `--format table|json`, `--filter status=<running|failed|stopped>`, `--verbose`；输出项目 ID、路径、状态、最近索引时间、worker PID；对应 `ProjectListRequest`。详细拆解见 @/docs/features/backend-service/project-list.md。
- **project status**：参数 `--project`, `--json`, `--fields`；展示单项目的 worker 状态、索引进度、watcher、最近请求、向量库版本；对应 `ProjectStatusRequest`。详细拆解见 @/docs/features/backend-service/project-status.md。
- **扩展命令（可选）**：保留 `project start/stop`, `project inspect`, `project sync` 等拓展点，对应未来协议；CLI 输出应保持一致的帮助文本/示例/退出码，并支持 `--json` 模式供脚本使用。
- **project restart**：参数 `--project`, `--wait`, `--grace`, `--force`, `--timeout`, `--reason`；执行单项目 stop→start，复用 restart 流程，映射 `RestartRequest(scope=project)`；详见 @/docs/features/backend-service/project-restart.md。
- **扩展命令（可选）**：保留 `project start/stop`, `project logs`, `project inspect`, `project sync` 等拓展点，对应未来协议；CLI 输出应保持一致的帮助文本/示例/退出码，并支持 `--json` 模式供脚本使用。

## 12. 项目/守护进程日志（`code-nav logs`）
详细规范见 @/docs/features/backend-service/logs.md，核心要求包括：
  - **统一 CLI 入口**：`code-nav logs --target <master|worker> --project <id|path> --since <duration|RFC3339> --limit <n> --follow --json --color auto|always|never --level <trace|...>`；worker 目标必须带项目引用，`--limit` 默认 500。
  - **时间/输出控制**：支持相对/绝对 `--since`、`--follow-interval`、文本（带颜色）与 JSON 两种输出模式，文本按 `[timestamp][LEVEL][source] message` 对齐，JSON 直接输出 `LogEvent`。
  - **日志聚合服务**：master 内置 `LogsService`（ring buffer + 可选落盘 + worker broadcast 通道），提供 `LogsHistory` 与 `LogsStream` RPC，负责过滤 `since/limit/level` 并向多个订阅者广播。
  - **错误与退出码**：统一 `ProjectNotFound/WorkerOffline/FollowTimeout/...` 错误结构，CLI 退出码 `0/2/3/1` 区分成功、项目不存在、follow 超时/中断与其他错误，`--json` 模式输出 `{ "error": { ... } }`。

