# 后端服务功能 + CLI 设计细则

## 1. 守护进程生命周期（code-navd）
- `code-navd start`：加载配置、校验 `.code-nav/`、初始化 SQLite / HNSW / 模型、启动监听（UDS/Named Pipe/TCP）、写入 PID/lock。
- `code-navd start` 细化职责：
  1. 配置解析与环境校验：流程详见 @/docs/features/backend-service/start-config.md。
  2. 单实例保障：检查 PID/lock 文件，防止重复实例；若已有进程运行则提示并退出或转为 `status`。
  3. 日志与监控：初始化 tracing/log（文件+STDOUT），可选启用 metrics/健康检查端口。
  4. 资源加载：打开 SQLite metadata、加载 HNSW/向量索引、预热 embedding 模型或远程客户端，初始化 `AppState`。
  5. 服务/任务启动：启动 RPC 监听（UDS/Named Pipe/TCP/HTTP）、注册路由，启动文件 watcher 与索引任务调度器。
  6. 索引状态：根据配置决定是否在启动时运行健康检查或触发增量索引，记录当前索引版本与队列。
  7. 生命周期管理：注册 SIGTERM/CTRL-C 处理器，确保 `stop` 可优雅关闭；启动成功后写入日志/控制台并保持守护模式。
- `code-navd stop`：通过控制 socket 或 PID 信号优雅退出，确保索引与队列安全落盘。细节见 @/docs/features/backend-service/stop.md。
- `code-navd restart`：顺序 stop → start，复用配置，提供 `--grace` 控制停机超时。
- `code-navd status`：返回运行状态、uptime、监听端点、任务数。
- `code-navd logs`（可选）：流式读取守护进程日志，支持 `--follow`/`--tail`。

## 2. 索引管理
- API：`/index/full`、`/index/incremental`。
- CLI：`code-nav index full [--watch]`、`code-nav index incremental [--paths foo.rs]`。
- 行为：调度 indexer 任务，写入 metadata.db 与向量索引，watcher 监听变更并触发增量任务。

## 3. 语义搜索
- API：`/search`。
- CLI：`code-nav search "<query>" --top-k 5 --lang rust --file-prefix src/`。
- 行为：文本 → embedding → HNSW 检索 → 结果排序 → 返回 `file/line/score/snippet`。

## 4. Goto 导航
- API：`/goto`。
- CLI：`code-nav goto "init http server" [--open <editor>]`。
- 行为：语义匹配符号，返回唯一位置及上下文，可触发编辑器打开。

## 5. 结构化列表
- API：`/list/classes`、`/list/methods`、`/list/files`。
- CLI：`code-nav list classes --filter Controller` / `list methods --limit 50` / `list files --lang ts`。
- 行为：基于 SQLite metadata 进行结构化查询，支持过滤、排序、分页。

## 6. 目录树
- API：`/tree`。
- CLI：`code-nav tree [path] --depth 3 --include-hidden`。
- 行为：返回目录树与文件类型信息，供 CLI 渲染。

## 7. 运行状态与信息
- API：`/status`、`/info`。
- CLI：`code-nav status`、`code-nav info`。
- 行为：`status` 报告索引进度、队列、健康；`info` 回传版本、协议、模型、配置。

## 8. 配置 & 模型管理（扩展）
- CLI：`code-nav config set <key> <value>`、`code-nav config get <key>`、`code-nav models list`、`code-nav models select <name>`。
- 行为：集中管理 embedding/向量后端与网络/API Key，必要时通知守护进程热更新。
