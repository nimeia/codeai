# `code-nav project restart` 任务说明

## 1. 目标与范围
- 为 **单个项目 worker** 提供 stop→start 重启能力，常用于索引卡死、配置变更或模型切换后自恢复。
- 支持 `--wait-ready` 模式，必要时可立刻返回 `Accepted` 并让 CLI 持续跟踪状态。
- 复用 `restart` 主流程：协议沿用 `RestartRequest`，scope=Project，master 负责调度，worker 负责优雅停机与自举。

## 2. CLI 入口与参数
1. 子命令：`code-nav project restart --project <path|id>`。
2. 支持参数：
   - `--wait-ready / --no-wait`：默认等待 worker 再次 Ready，可通过 `--no-wait` 提前返回。
   - `--timeout <sec>`：`--wait-ready` 的最大等待时长，默认 60s。
   - `--grace <sec>`：传递给 worker 的优雅停机宽限期，默认与 `project stop` 相同（例如 10s）。
   - `--force`：宽限期结束后若 worker 未退出则强制杀死，默认 false。
   - `--reason <text>`：写入事件日志，便于审计（如 "watcher stuck"）。
   - `--json`：输出结构化阶段与结果。
3. 交互流程：
   - CLI 构造 `ProjectRestartRequest`，包含参数和 `project_ref`（路径或 ID），发送到 master 控制 socket。
   - master 立即回复 `ProjectRestartResponse { accepted: bool, state, message }`。
   - 若 `--wait-ready`，CLI 循环调用 `project status`（或新的 `RestartWatch` API）直到状态进入 `Ready`/`Failed`/超时。

## 3. 协议结构
- `ProjectRestartRequest { project_ref, grace_secs, force, wait_ready, wait_timeout, reason }`。
- `ProjectRestartResponse { project_id, state, accepted_at, completed_at, message }`。
- `ProjectRestartState`：`Accepted | Stopping | Stopped | Starting | Ready | Failed { reason } | NotFound | Busy`。
- 协议要求：
  - CLI 发送 `wait_ready=false` 时 master 仍需返回初始状态和 `restart_task_id` 以便后续轮询。
  - 失败时 `reason` 字段描述 stop/start 哪个阶段失败，并在 master 日志中落盘。
  - 与 master 全局 `RestartRequest` 兼容：`Request::Restart(RestartScope::Project { project_id, options })`。

## 4. Master 调度职责
1. **校验与排队**：
   - 根据 `project_ref` 查 registry，若不存在返回 `NotFound`。
   - 若项目已处于 `Restarting/Removing/Stopping`，返回 `Busy` 并给出提示。
   - 将任务写入 `ProjectRestartQueue`，包含 stop/start 参数、reason、task_id。
2. **停止阶段**：
   - 通知 supervisor 对应 worker 执行 `StopWorker`，携带 `grace_secs` 与 `force`。
   - 在 registry / `ProjectStateStore` 中把 runtime 状态置为 `Restarting(Stopping)`，对 `project list/status` 可见。
   - 监听 worker 退出事件；如超时触发强制 kill 并记录原因。
3. **启动阶段**：
   - 复用 autostart 逻辑，使用之前的项目配置启动 worker。
   - 成功 ready 后更新 registry（`worker_pid`、`socket`、`last_restart_at`）。
   - 若启动失败，将 `runtime_state` 置为 `Failed` 并保留最后一次错误，供 CLI 显示。
4. **任务完成通知**：
   - 在 `ProjectRestartResponse` 中返回 `completed_at`（若已完成）和当前状态。
   - 若 CLI 选择 wait，master 可支持 `RestartTaskWatch` streaming；第一版允许 CLI 轮询 `project status`。

## 5. Worker/Supervisor 状态机
1. 新增事件：`WorkerEvent::Restarting`, `WorkerEvent::Restarted`, `WorkerEvent::RestartFailed`。
2. Worker 接到 `StopWorker`：
   - 标记 `RuntimeState=Stopping`，暂停新请求。
   - Flush 索引/embedding 任务，通知 watcher 停止，保存必要的 checkpoint。
   - 在宽限期内优雅退出；若 `force=true` 则 supervisor 发送 SIGKILL。
3. Supervisor 负责在 worker 退出后根据任务类型决定是否拉起新实例，并在启动成功/失败时上报事件。
4. 新 worker `Ready` 后发送心跳，包含 `restart_reason`、`restart_count`，master 用于 CLI 展示。

## 6. CLI 输出与阶段反馈
1. 文本模式：
   - 立即打印 `Project <id> restart accepted (task=<id>)`。
   - 若 `--wait-ready`：
     ```
     [Stopping] Waiting for worker to exit (pid=12345)...
     [Stopped] Worker exited after 2.1s, restarting...
     [Starting] Worker pid=23456 socket=/tmp/... ready? pending index boot...
     [Ready] Restart completed in 7.4s.
     ```
   - 失败示例：`[Failed] Worker failed to start: index bootstrap timeout`。
2. JSON 模式：输出阶段数组：`[{"state":"Stopping","timestamp":...}, ...]`，并包含最终 `result`（`Ready/Failed/Timeout`）。
3. 超时处理：若等待超过 `--timeout`，CLI 退出码为 3，并提示使用 `code-nav project status --project <...>` 查看实时状态。

## 7. 错误处理与退出码
- 0：成功完成或 `--no-wait` 接受任务。
- 2：项目不存在 / 已在重启或被删除。
- 3：等待超时。
- 4：停止阶段失败（例如 worker 无响应且 force=false）。
- 5：启动阶段失败或 supervisor 无法创建新 worker。
- CLI 需将 master 返回的 `message/reason` 打印到 STDERR，并在 JSON 模式中写入 `error` 字段。

## 8. 协同与验收
1. `project add/list/status` 共用 `RuntimeStateSummary`，当状态为 `Restarting` 时可指向 `project restart --wait`。
2. `project remove` 必须拒绝与 `restart` 并发执行（或先取消重启任务）。
3. 测试清单：
   - 单项目正常重启：worker 停止→启动→CLI wait 成功。
   - 重启失败：模拟索引初始化错误，确认状态为 Failed 且 CLI 输出原因。
   - 超时：将启动卡住，验证 CLI 超时退出并提示后续操作。
   - 并发：对多个项目发起 restart，验证 queue 限制与状态隔离。
   - 强制 kill：worker 卡住不退出，`--force` 生效并记录强制原因。
