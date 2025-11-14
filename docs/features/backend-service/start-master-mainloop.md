# `code-navd start` 第七步：主循环与信号处理

## 1. 信号注册
- **Unix/macOS**：监听 `SIGTERM`, `SIGINT`，可选 `SIGHUP` 用于重新加载配置。
- **Windows**：使用 `tokio::signal::ctrl_c()` 或 `SetConsoleCtrlHandler` 捕获控制事件。
- 处理方式：收到终止信号后向主循环发送 `ShutdownRequest`，触发优雅停机。

## 2. 主循环职责
- 运行在 async runtime（推荐 tokio）中，`select!` 监听以下事件：
  - 控制端点连接：接受 CLI 请求并交由任务处理。
  - Worker 心跳/状态更新：worker 周期性发送 `Heartbeat { project_id, status }`。
  - 自动启动/空闲回收定时器。
  - 信号/内部命令（如 `ShutdownRequest`、`ReloadConfig`）。
- 主循环维护共享状态（registry、启动队列、活动 worker 映射），通过 `Arc<RwLock<...>>` 管理。

## 3. 请求处理
- 控制端点每个连接由独立任务处理，将请求解析为 `MasterCommand`：
  - `ProjectAdd`, `ProjectRemove`, `ProjectStatus`, `ProjectList`
  - `RouteRequest`（将 CLI 的 search/list/goto/index/status/info 请求转发到对应 worker）
  - `Shutdown`, `Reload`
- 任务通过 channel（`mpsc`）将命令发送给主循环线程，由主循环执行，保证状态一致。

## 4. Worker 监控
- worker 启动后向 master 注册（`WorkerHello`），提供 `pid`, `socket`, `capabilities`。
- worker 定期发送 `Heartbeat`（包含索引进度、任务数、最近请求时间）。
- 若心跳超时（超过 `heartbeat_timeout`）：
  - 记录 WARN，标记 worker 状态为 `Unresponsive`。
  - 可触发重启策略或通知 CLI。
- worker 正常退出时发送 `ShutdownAck`；master 更新 registry 并释放资源。

## 5. 自动启动与空闲回收
- 主循环维护 `autostart_queue` 与 `idle_reclaimer`：
  - 定时检查 `autostart_queue`，在 `max_concurrent_starts` 限制下启动 worker。
  - 定期扫描 worker 最后活动时间，若超过 `idle_timeout` 且无挂起任务，发送 `Shutdown`。

## 6. 优雅停机流程
1. 接收到 `ShutdownRequest`（信号或 CLI `stop`）。
2. 设置 `state = ShuttingDown`，停止接受新 CLI 请求（监听器返回“shutting down”）。
3. 向所有 worker 发送 `Shutdown { grace_period }`。
4. 等待 worker 退出：通过 `JoinHandle`/`Child` 或心跳确认，超时则根据 `--force` 决定是否终止进程。
5. 写回 registry（更新 `last_running=false`, `last_state=Stopped`），删除 PID/锁文件。
6. 关闭日志、metrics，最终退出进程。

## 7. 错误与恢复
- 主循环 panic：记录 ERROR 并决定是否重启（可配置 `restart_on_panic`）。
- 信号 handler 应尽量轻量，仅发送消息到主循环，避免在 handler 中执行复杂逻辑。
- Worker 崩溃：主循环接收到退出事件或心跳超时后：
  - 更新 registry 状态为 `Crashed`。
  - 判断是否自动重启（与 `autostart`/`auto_restart` 配置挂钩）。

## 8. 调试与可观测性
- 支持 `code-navd logs --follow` 订阅 master 日志流（通过控制端点提供流式输出）。
- 主循环可定期输出 summary 日志（例如每 60s：“workers=3 running, queue=1, requests/s=15”）。
- 监控指标：`master_uptime`, `active_workers`, `command_failures_total`, `shutdown_initiated_total`。

完成主循环与信号处理设计后即可进入最终启动反馈步骤。
