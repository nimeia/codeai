# `code-nav project remove` 任务说明

## 1. 目标与范围
- 允许用户安全地将某个项目从 master registry 中移除，默认进行优雅停机、清理运行态并根据选项删除工作目录。
- 支持 `--keep-data`、`--force`、`--grace` 等参数，确保可以控制保留 `.code-nav/` 索引、在宽限期内优雅退出或在超时时强制回收。
- CLI、协议、master、worker/supervisor 必须协作，提供一致的状态回报与可观测性，避免遗留僵尸 worker 或脏数据。

## 2. CLI → Master 交互流程
1. CLI 解析参数：`--project <path|id>`（必填）、`--force`、`--keep-data`、`--grace <seconds>`、`--json`、`--yes`（无需二次确认）。
2. CLI 将解析结果序列化为 `ProjectRemoveRequest`，字段包含定位项目的路径或 ID、强制/保留策略、宽限期、输出模式与确认标记。
3. 请求通过控制 socket 发送至 master，master 立即返回 `ProjectRemoveResponse`，内容包含匹配的项目 ID/路径、停机状态与清理结果。
4. CLI 根据 `remove_state`（`Accepted|NotFound|Rejected|Failed`）以及 `worker_shutdown`（`Draining|Stopped|ForceKilled|NotRunning`）渲染用户提示，JSON 模式下输出结构化结果并设置退出码。

## 3. Master 职责拆解
1. **定位与校验**
   - 统一在 registry 中支持通过 `project_id` 或 `project_root` 查找项目；若二者都提供则需匹配验证。
   - 校验项目当前状态（是否仍在删除中、是否有未完成的索引任务），必要时返回 `Rejected`。
2. **停机编排**
   - 通过 worker supervisor 下发 `ShutdownProject { project_id, grace_secs, force }` 事件；若 worker 未运行直接标记 `NotRunning`。
   - 当 CLI 未指定 `--force` 时，先进入 `Draining` 状态，等待 worker 在 `grace_secs` 内报告 `Stopped`；超时后自动转入 `ForceKill`，并在响应中注明。
3. **清理 registry 与运行态**
   - 停机完成后删除 registry entry，或者在 `--keep-data` 时仅标记为 `Removed` 并保留历史记录供 `project list --all` 查询。
   - 清理 runtime 目录（socket、pid、lock、pending 队列项），并在 `--keep-data=false` 时删除 `project_root/.code-nav/`（需要权限检测与失败回报）。
4. **状态与错误处理**
   - 在 `ProjectStateStore` 中记录 `Removing` 状态，供 `project list/status` 查询。
   - 失败时写入 `last_error` 并允许 CLI 指示用户使用 `--force` 或手动清理，避免 registry 残留半删除条目。

## 4. Worker / Supervisor 协作
1. **Supervisor 行为**
   - 接收 `ShutdownProject` 任务后向目标 worker 发送 `WorkerShutdown { grace_secs }`，并等待 `WorkerEvent::Stopped`。
   - 超时或 worker 无响应时执行 `force_kill(pid)`，并将结果回传 master（`ForceKilled`/`Failed { reason }`）。
2. **Worker 响应**
   - 监听 shutdown 事件后停止接收新的 CLI 请求，flush 索引任务、watcher 与 embedding 资源，写入 `shutdown.log` 以辅助调试。
   - 根据 `keep_data` 标志决定是否清理 `.code-nav/` 内的缓存（索引文件、临时目录），并确保在完成后上报 `Stopped`。
3. **数据清理钩子**
   - 当 master 需要删除 `.code-nav/` 时，可复用 worker 的 `cleanup_project_files` helper（若 worker 已退出则 master 直接操作但需要镜像 worker 的权限校验）。

## 5. 协议结构
- `ProjectRemoveRequest { project_ref: ProjectRef, force: bool, keep_data: bool, grace_secs: Option<u32>, assume_yes: bool, format: OutputFormat }`
- `ProjectRef = ByPath(PathBuf) | ById(String)`；CLI 支持自动在路径不存在时尝试 ID 匹配。
- `ProjectRemoveResponse { state: RemoveState, project_id: Option<String>, project_root: Option<PathBuf>, worker_shutdown: WorkerShutdownState, data_cleanup: DataCleanupState, message: String }`
- `RemoveState = Accepted | NotFound | Rejected | Failed`。
- `WorkerShutdownState = NotRunning | Draining | Stopped | ForceKilled | Failed { reason }`。
- `DataCleanupState = SkippedKeepData | Deleted | DeleteFailed { reason }`。

## 6. CLI 输出规范
- 默认文本示例：
  ```
  Project removed: <project_id>
  Worker: stopped (grace=10s)
  Data: kept (use --keep-data=false to delete)
  ```
- JSON 输出需包含 `state`, `worker`, `data` 字段，脚本可检查 `state != "Accepted"` 或 `worker == "Failed"` 时决定是否重试。
- 退出码：0（成功删除）、2（未找到或被拒绝）、3（worker/data 清理失败但 registry 已更新）、1（其他错误）。

## 7. 状态与幂等
- 多次调用 `project remove` 应保持幂等：第二次调用在 registry 条目缺失时返回 `NotFound` 并提示。
- 如果上一次删除失败并遗留 `Removing` 状态，CLI 再次调用应能检测到并提供 `--force` 指引。
- `project list/status` 在删除期间展示 `Removing`，并附带 `worker_shutdown` 进度（如 `Draining 3s/10s`）。

## 8. 验收清单
1. CLI：help/usage、参数解析、确认提示、JSON/文本输出、退出码。
2. Protocol：`ProjectRemoveRequest/Response` 序列化测试，`RemoveState/WorkerShutdownState/DataCleanupState` 枚举保持兼容。
3. Master：registry 查找/删除单测、`Removing` 状态迁移、`keep-data` 与清理失败路径、force kill 分支。
4. Worker/Supervisor：`WorkerShutdown` 处理测试（优雅、超时、失败），`cleanup_project_files` 行为覆盖 keep/delete。
5. 集成：启动 master+worker → 执行 `project remove`（含 keep-data/force 组合）→ 验证 CLI 输出、registry/文件系统状态、重复调用幂等。
