# `code-navd restart` 第六节：测试与验收

## 1. CLI 层
- **参数解析**：单元测试覆盖 `--wait/--no-wait`、`--grace`、`--force`、`--timeout`、`--project`（单个/多个）、`--reason`、`--all`/`--failed` 等组合。
- **交互提示**：模拟交互式环境，验证默认确认提示、`--yes` 生效、非交互默认拒绝。
- **输出与退出码**：使用 mock server 或断言写入器，检查成功、Busy、Timeout、连接错误等路径的 stdout/stderr 与 exit code 映射。

## 2. Protocol 层
- `RestartRequest/RestartResponse` 序列化与反序列化测试，覆盖所有 `RestartScope`、`RestartState`、`Option` 字段组合，确保向后兼容（未知字段忽略）。
- `wait_ready` 行为：测试 server 在 `wait_ready=true` 时阻塞至完成，在 `false` 时立即返回 `accepted`。
- 错误响应：模拟 `ErrorCode::Busy/NotIndexed/PermissionDenied/Unsupported/InternalError`，验证 CLI 解析与提示。

## 3. Server 单元测试
- **状态机**：master 与 worker 重启状态转换（`Running → Restarting → Completed/Failed`）、`force` 分支、失败回滚。
- **Worker 模拟**：mock watcher/indexer/worker 句柄，触发 stop 超时、force、start 失败，验证 registry 与日志更新。
- **批量处理**：传入多个项目，验证顺序/并发、失败聚合、部分成功时的反馈。

## 4. 集成测试
- **Master 重启成功**：启动 test server + worker，执行 `code-nav restart`，确认 master 停止并重新 ready，CLI 输出与 exit code 为 0。
- **Master 重启失败**：模拟 worker 无法停止或 start 失败，CLI 获取 `RestartState::Failed`，`status` 显示失败原因。
- **Worker 重启成功**：对单项目执行，验证 stop→start、registry `last_restart_*` 字段更新。
- **Worker 重启超时/force**：制造停不下的 worker，验证 `--force` 生效、日志记录。
- **并发/批量**：同时重启多个项目，确保无竞态（registry 锁、自动启动冲突）。
- **重启期间请求**：在项目 `Restarting` 时发起 search/index，确认得到 `ErrorCode::Busy` + “重启中”提示。

## 5. 回归验证
- Stop/Start/Restart 组合运行：确保锁文件、控制端点、registry 状态在多次操作后仍一致。
- 日志与指标：断言 `restart_request/restart_complete` 日志出现，metrics `restart_total`, `restart_fail_total`, `restart_duration_seconds` 更新。
- 幂等性：连续执行 `restart`（目标正在重启或刚完成），确保 CLI 获得 `AlreadyRestarting` 或 `Completed`，不视为失败。
