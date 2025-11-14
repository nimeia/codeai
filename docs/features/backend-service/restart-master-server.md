# `code-navd restart` 第四节：Server 处理流程

## 1. 请求分派与状态检查
- 控制端点接收 `RestartRequest` 后，由 master 主循环串行处理，保证 registry/state 一致。
- 在执行前检查全局状态：
  - 若 master 处于 `Stopping/Restarting`，直接返回 `RestartState::AlreadyRestarting`。
  - 若目标项目不存在或状态未知，返回 `ErrorCode::NotIndexed`。
- 所有状态变更写入 `state::AppState` 并对 `status` 命令可见。

## 2. Master 重启流程
1. **状态切换**：`app_state.master = Restarting { started_at, reason, force }`；对外 `status` 报告 `Restarting`。
2. **停止接入层**：暂停新的 CLI 请求（监听器拒绝或返回“重启中”），防止新任务进入。
3. **关闭 worker**：
   - 遍历 registry，向每个 worker 发送 `WorkerShutdown { grace_secs, force }`。
   - 等待 ack/退出，期间记录剩余 worker 数、超时与强制操作。
4. **自停与重启**：
   - 清理控制 socket、lock/pid。
   - 在 CLI/systemd 模式下：master 进程退出，由外层脚本重新执行 `code-navd start`；若实现自重启，可在同进程执行 stop→start。
5. **Ready 通知**：
   - 对 `wait_ready=true` 的请求，需要在新 master 完成启动后返回 `RestartState::Completed`。
   - 若 master 退出导致无法直接响应，可在停止前持久化“重启 token”，新实例启动时读取并发回（可选扩展）。

## 3. 项目 worker 重启流程
1. **验证项目**：查 registry；不存在 → `NotIndexed`。
2. **标记状态**：`project.state = Restarting`，CLI 对该项目的请求返回“重启中”或 `ErrorCode::Busy`。
3. **停止 worker**：
   - 通过 IPC 发送 `Shutdown { grace_secs, force }`，等待 ack/PID 退出。
   - 若 `force=true` 且超时，发送 `SIGTERM`→`SIGKILL`（或 Windows 等价）。
4. **启动 worker**：
   - 复用 autostart 的 spawn 流程，构建启动命令、等待 ready（ping socket 或 worker Ready 消息）。
   - 更新 registry：`last_state=Running`, `last_restart_at`, `last_restart_reason`, `last_restart_duration`.
5. **返回结果**：
   - `wait_ready=true`：在 ready 后返回 `RestartState::Completed`。
   - `wait_ready=false`：stop 阶段完成即可返回 `Accepted`，后台继续启动并通过日志/状态可见。
6. **错误处理**：
   - stop 阶段失败：返回 `RestartState::Failed` 或 `ErrorCode::Busy`（若未 force）。
   - start 阶段失败：记录日志、registry 标记 `Failed`，向 CLI 返回 `Failed` 并附 message。

## 4. 批量执行
- 当 `scope` 包含多个项目：
  - 主循环根据 `max_parallel_restarts` 配置并发处理；默认顺序逐个。
  - 响应可包含 `Vec<ProjectRestartResult>`，每个项目独立 `state`/`message`。
  - CLI exit code 根据任何失败/超时决定（任一失败→2；全部成功→0）。

## 5. 状态、日志与协作
- `state::AppState`：记录 master/项目级 `Restarting` 状态、开始/结束时间、发起人、是否 force。
- Registry：新增 `last_restart_{at,reason,result,duration}` 字段。
- 日志事件：
  - `restart_request { scope, projects, force, reason }`
  - `restart_stop_phase { scope, remaining_workers, elapsed_ms }`
  - `restart_start_phase { project_id, attempt }`
  - `restart_complete { scope, success, duration_ms }`
  - `restart_failed { scope, error }`
- 与自动启动/回收协同：
  - 重启期间暂停自动回收或其它重启任务，避免竞争。
  - watcher/indexer 在 stop 前 checkpoint 进度，确保重启后能恢复。
  - CLI 对 `Restarting` 状态的项目发起 search/index 等命令时，返回 `ErrorCode::Busy` + “项目重启中”提示。
