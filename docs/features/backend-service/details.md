# 后端服务功能 + CLI 设计细则

## 1. Master 服务（code-navd）
- 架构总览参考 @/docs/features/backend-service/master-worker.md；`code-navd` 作为 master 负责 CLI 请求入口与 worker 生命周期。
- 主命令：
  - `code-navd master start|stop|status`：控制 master 生命周期，参数详见 master-worker 文档。
  - `code-navd project start|stop|status|restart|list`：通过 master 管理项目 worker。
  - `code-navd check`：执行配置/环境自检。
- Master 核心职责：
  1. 统一控制端点：创建 `master.sock`（UDS）或 `code-nav-master`（Named Pipe），为 CLI 提供单一入口。
  2. 项目 registry：记录 `project_root`、worker PID、IPC 地址、索引状态、最近活动时间。
  3. 请求路由：接收 CLI 命令，根据 `project` 参数启动/选择 worker，转发 `search/goto/list/tree/index` 等 RPC 并返回结果。
  4. 监控与调度：监听 worker 存活、崩溃、闲置；按策略回收或重启。
  5. 优雅停机：master 退出前向所有 worker 发送 `Shutdown`，等待其完成后再关闭自身。

## 2. 项目 worker（code-nav-worker）
- 每个项目由独立 worker 承担索引/搜索/监听逻辑，可视为旧版 `code-navd` 的功能收敛。
- `code-nav-worker start` 细化职责：
  1. 配置解析与环境校验：流程详见 @/docs/features/backend-service/start-config.md。
  2. 单实例保障：检查项目级 PID/lock 文件，防止重复 worker。
  3. 日志与监控：初始化 tracing/log（文件+STDOUT），可选 metrics。
  4. 资源加载：打开 SQLite metadata、加载 HNSW/向量索引、预热 embedding/远程客户端，初始化 `AppState`。
  5. 服务/任务启动：启动与 master 的 IPC 监听（UDS/Named Pipe/TCP）、注册 RPC 路由，启动文件 watcher 与索引调度器。
  6. 索引状态：根据配置执行健康检查或触发增量索引，记录索引版本与队列。
  7. 生命周期管理：注册 SIGTERM/CTRL-C 处理器，确保 `stop` 可优雅退出；向 master 汇报启动成功。
- `code-nav-worker stop`：通过 IPC 或 PID 信号优雅退出，保障索引任务安全落盘；细节类似 @/docs/features/backend-service/stop.md。

## 3. 索引管理
- API：`/index/full`、`/index/incremental`。
- CLI：`code-nav index full --project <path> [--watch]`、`code-nav index incremental --project <path> [--paths foo.rs]`（由 master 路由至相应 worker）。
- 行为：调度 indexer 任务，写入 metadata.db 与向量索引，watcher 监听变更并触发增量任务。

## 4. 语义搜索
- API：`/search`。
- CLI：`code-nav search "<query>" --project <path> --top-k 5 --lang rust --file-prefix src/`（CLI → master → worker）。
- 行为：文本 → embedding → HNSW 检索 → 结果排序 → 返回 `file/line/score/snippet`。

## 5. Goto 导航
- API：`/goto`。
- CLI：`code-nav goto "init http server" --project <path> [--open <editor>]`。
- 行为：语义匹配符号，返回唯一位置及上下文，可触发编辑器打开。

## 6. 结构化列表
- API：`/list/classes`、`/list/methods`、`/list/files`。
- CLI：`code-nav list classes --project <path> --filter Controller` / `list methods --project <path> --limit 50` / `list files --project <path> --lang ts`。
- 行为：基于 SQLite metadata 进行结构化查询，支持过滤、排序、分页。

## 7. 目录树
- API：`/tree`。
- CLI：`code-nav tree --project <path> [path] --depth 3 --include-hidden`。
- 行为：返回目录树与文件类型信息，供 CLI 渲染。

## 8. 运行状态与信息
- API：`/status`、`/info`。
- CLI：`code-nav status --project <path>` 查询单个 worker；`code-nav info --project <path>` 查看项目信息；`code-nav project status` 查看 master + 所有 worker。
- 行为：`status` 报告索引进度、队列、健康；`info` 回传版本、协议、模型、配置。

## 9. 配置 & 模型管理（扩展）
- CLI：`code-nav config set <key> <value>`、`code-nav config get <key>`、`code-nav models list`、`code-nav models select <name>`（可带 `--project` 针对特定 worker，或不带针对 master/global）。
- 行为：集中管理 embedding/向量后端与网络/API Key，必要时通知守护进程热更新。
