# `code-nav project status` 任务说明

## 1. 目标与范围
- 输出 **单个项目** 的运行时快照，覆盖 registry 元数据 + worker/索引/监听器指标。
- 支持一次性查询与 `--watch` 连续刷新，方便排查初始化或索引卡住的问题。
- 复用 `project add/list` 的状态结构，保持字段含义一致。

## 2. CLI → Master 交互流程
1. CLI 参数：`--project <path|id>`（必填）、`--json`、`--fields <comma-list>`、`--watch [interval]`、`--follow-events`（可选）。
2. 解析后构造 `ProjectStatusRequest { project_ref, fields, format, follow }` 并发送至 master 控制 socket。
3. Master 查找 `project_ref` 对应条目，聚合 registry + runtime state + 最近 worker 心跳，封装 `ProjectStatusResponse { project_id, project_root, registry, runtime, indexing, watcher, embedding, diagnostics }`。
4. CLI 根据 `format` 渲染：
   - 默认文本：展示主字段（State/Worker PID/Last Seen/Index Revision/Pending Jobs/Watcher/Last Error）。
   - `--json`：原样输出响应结构。
5. `--watch` 时 CLI 定时重复请求，或在 master 提供的 streaming 模式下持续读取。

## 3. Master 状态聚合职责
1. **状态存储接口**
   - 在 `ProjectStateStore` 中新增 `get_snapshot(project_id)`，返回 `ProjectStatusSnapshot`。
   - Snapshot 结构包含：
     ```
     struct ProjectStatusSnapshot {
         registry: RegistryEntry,
         runtime_state: RuntimeStateSummary,
         worker: Option<WorkerSummary>,
         indexing: Option<IndexingSummary>,
         watcher: Option<WatcherSummary>,
         embedding: Option<EmbeddingSummary>,
         diagnostics: DiagnosticsSummary,
     }
     ```
2. **数据来源**
   - Registry：`registry.json` 中的路径、策略、autostart/watch 标记。
   - Runtime：worker supervisor 维护的 `state`（`Pending/Starting/Ready/Failed/Stopped`）、`last_transition`, `restart_count`。
   - Worker：最近一次心跳的 `pid`, `socket_path`, `uptime`, `rss`, `cpu`。
   - Indexing：`index_revision`, `last_full`, `last_incremental`, `pending_jobs`, `throttled_reason`。
   - Watcher：`mode`, `last_event_at`, `backlog`, `is_paused`。
   - Embedding：`provider`, `model`, `cache_warm`, `last_error`。
   - Diagnostics：最近 5 条错误/事件、`last_request`、`last_request_error`。
3. **缺省值**
   - 若某块信息缺失，填 `None/Unknown` 并在 CLI 渲染为 `-`；避免 panic。
4. **错误处理**
   - 未找到项目：返回 `ProjectNotFound` 状态 + message，CLI 退出码非零。
   - 状态聚合出错：`InternalError`，附调试信息。

## 4. Worker / Supervisor 指标上报
1. Worker 心跳消息扩展：`StatusReport { runtime, indexing, watcher, embedding, diagnostics }`。
2. Supervisor 负责在 worker 重启/失败时主动推送事件，防止状态长期滞留。
3. `runtime.state` 统一枚举：`Pending | Starting | Ready | Degraded | Failed { reason } | Stopped`。
4. `indexing` 字段：
   - `current_task`（`None | Full | Incremental { paths }`）。
   - `progress`（0-100）、`queued_jobs`, `last_success_at`, `last_error`。
5. `watcher` 字段：模式（`native/fsnotify/polling`）、`last_event`, `pending_events`, `errors`。
6. `embedding` 字段：`provider`, `model`, `last_refresh_at`, `cache_warm`, `errors`。
7. `diagnostics`：`recent_errors`（按时间排序）、`recent_requests`（带耗时）、`notes`（自由文本）。

## 5. CLI 输出与过滤
1. 默认文本格式：
   ```
   Project: <project_id>  (<project_root>)
   State: Ready (since 2024-03-01T10:22:33Z)
   Worker: pid=12345 socket=/tmp/code-nav/project-foo.sock uptime=12m cpu=4.3% rss=512MB
   Indexing: revision=42 full=2024-02-28T.. incremental=2024-03-01T.. pending=0 progress=100%
   Watcher: fsnotify last_event=2024-03-01T10:20:11Z backlog=0
   Embedding: provider=local-candle model=all-MiniLM cache=warm
   Diagnostics: last_error=none recent_requests=search(230ms) goto(90ms)
   ```
2. `--fields`：支持 `state,worker,indexing,watcher,embedding,diagnostics`，CLI 只渲染选定部分。
3. `--json`：输出 `{ "project": { ... }, "state": { ... } }`；兼容 `jq`。
4. `--watch [interval]`：默认 2s；CLI 清屏重绘或逐行刷新，直到 Ctrl+C。
5. `--follow-events`：可选的 streaming 模式，master 在状态变化时推送事件，CLI 实时打印（实现可后续扩展）。

## 6. 与其他命令的协同
- `project add`：在注册成功后提示“使用 `project status --project ...` 跟进”，两者共享 `WorkerInitState`。
- `project list`：使用相同的 `RuntimeStateSummary` 字段，用户可从列表跳到 status 查看详情。
- `project remove/restart`：status 需能识别 `Removing/Restarting` 的过渡状态，并提示“操作中”。

## 7. 验收标准
1. **CLI**：参数解析单测、`--json` 与文本输出快照、`--watch` 模式刷新逻辑、字段过滤正确。
2. **协议**：`ProjectStatusRequest/Response` 序列化/反序列化测试，`RuntimeStateSummary` 与共享结构保持向后兼容。
3. **Master**：`ProjectStateStore::get_snapshot` 单测（缺失字段、worker 离线、索引信息缺失）、速率限制/缓存策略验证。
4. **Worker/Supervisor**：心跳/事件在 ready、索引进行中、watcher 错误等场景下正确上报，master 成功聚合。
5. **E2E**：
   - 正常：`project add` → 等待 worker ready → `project status --project <path>` 显示完整信息。
   - 错误：查询不存在项目 → 友好错误信息。
   - 观察：运行 `--watch`，在 worker 重启或索引任务进度变化时看到实时更新。
