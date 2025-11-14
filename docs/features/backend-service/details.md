# 后端服务功能 + CLI 设计细则

## 1. 总览任务
1. Master 启动流程（`code-navd start`）：定义配置结构、控制端点创建、前台/后台模式、日志初始化。
2. 项目注册与 worker 管理（`project add/remove/list/status/restart`）：设计 registry、worker 启停、自动重启/回收策略。
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
  - 优雅停机：master 退出前发出 `Shutdown` 给所有 worker，等待其完成后再关闭自身。

### 2.1 `code-navd start` 详细流程
1. **配置解析**：流程详见 @/docs/features/backend-service/start-master-config.md；读取默认配置、环境变量与 CLI 参数，生成 `MasterConfig` 并校验控制端点与 `.code-nav/` 目录。
2. **单实例检测**：详见 @/docs/features/backend-service/start-master-single-instance.md；通过 `master.lock/master.pid` 防止重复实例，检测僵尸并输出提示。
3. **日志与监控初始化**：详见 @/docs/features/backend-service/start-master-logging.md；根据配置设置 tracing（级别、文件/STDOUT、轮转）并可选启动 metrics/health 服务。
4. **控制端点建立**：创建 UDS（0600 权限）、Named Pipe 或 TCP loopback 监听，准备接受 CLI 请求（复用 master RPC 协议）。
5. **项目 registry 恢复**：读取 `~/.code-nav/projects/registry`，加载历史项目，检测 worker 是否存活，更新状态并清理僵尸锁。
6. **自动启动策略**：根据配置的 `autostart` 或 `auto_resume` 启动各项目 worker（spawn 子进程），等待 worker ready（握手或 socket 就绪）。
7. **主循环与信号处理**：注册 SIGTERM/SIGINT（或 Windows 控制事件），优雅停机时向所有 worker 发送 `Shutdown`；主循环处理 CLI 请求路由、worker 状态更新、闲置回收。
8. **启动反馈**：成功后输出“master running”日志/提示；前台模式保持控制台输出，后台模式可写 readiness 文件或在 CLI 中反馈成功。

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
- Worker 停止流程参照 @/docs/features/backend-service/stop.md（通过 IPC 指令或 PID 信号优雅退出）。

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

## 9. 运行状态与信息
- API：`/status`、`/info`。
- CLI：`code-nav status --project <path>` 查询单个 worker；`code-nav info --project <path>` 查看项目信息；`code-nav project status` 查看 master + 所有 worker。
- 行为：`status` 报告索引进度、队列、健康；`info` 回传版本、协议、模型、配置。

## 10. 配置 & 模型管理（扩展）
- CLI：`code-nav config set <key> <value>`、`code-nav config get <key>`、`code-nav models list`、`code-nav models select <name>`（可带 `--project` 针对特定 worker，或不带针对 master/global）。
- 行为：集中管理 embedding/向量后端与网络/API Key，必要时通知 master/worker 热更新。
