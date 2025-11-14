# `code-navd stop` 第三步：守护进程关机流程

## 1. 状态机
- `Running` → 接到 stop 请求 → `Stopping(grace_deadline)`。
- `Stopping` → worker 全部退出 → `Stopped`。
- 若 `force=true` 且宽限期超时 → 进入 `Forcing` 状态，发出强制终止信号，随后进入 `Stopped`。
- 状态写入 `state::AppState` 并暴露给 `status` 命令。

## 2. 操作顺序
1. **停止接入层**：停止接受新的 CLI 请求（监听器返回“shutting down”），防止再有 search/index 指令进来。
2. **广播关闭**：
   - watcher：停止文件监听，清空事件队列。
   - indexer/embedding：设置信号 `cancel`，让长任务尽快返回。
   - 项目 worker：逐个发送 `WorkerCommand::Shutdown { grace }`。
3. **等待**：
   - 通过 `JoinHandle`、worker 心跳或 RPC `ShutdownAck` 检查完成状态。
   - 在 `grace_period` 内循环检查剩余 worker 数；每次更新写入日志和 `StopResponse.message`。
4. **强制**（可选）：若 `force=true` 且宽限期后仍有 worker，发送 `SIGTERM`→`SIGKILL`（Unix）或 `GenerateConsoleCtrlEvent`→`TerminateProcess`（Windows）。同时记录强制原因。
5. **清理与退出**：
   - 删除 `master.lock/master.pid`、控制 socket/pipe。
   - Flush metrics、日志、数据库连接。
   - 设置退出码（0=成功，1=强制/失败）并调用 `std::process::exit`.

## 3. 资源回收细节
- **索引任务**：若允许中断，需 checkpoint 当前进度（写入 `metadata.db`），以便下一次重建；否则在 stop 前阻止新的索引请求。
- **向量库/SQLite**：调用 `Connection::close` 或 drop，确保无 WAL 残留；必要时执行 `PRAGMA wal_checkpoint`.
- **Embedding 模型**：释放 GPU/内存句柄，等待推理线程结束。

## 4. 日志与可观测性
- 关键事件：
  - `shutdown_request { from=<socket>, force, timeout }`
  - `shutdown_progress { remaining_workers, elapsed_ms }`
  - `shutdown_force { pid, reason }`
  - `shutdown_complete { elapsed_ms }`
- Metrics：`shutdown_total`, `shutdown_force_total`, `shutdown_duration_seconds`.
- CLI 可在 stop 期间 tail 日志获取更详细进度。

## 5. 回调与钩子
- 在 server 内部暴露 `shutdown::register_hook(FnOnce)`，允许模块追加自定义清理逻辑（如释放缓存文件、写入快照）。
- 钩子按注册顺序或逆序执行（推荐逆序，类似栈），每个钩子超时后记录 WARN 但不阻塞整体退出。

## 6. 测试策略
- 单元测试：状态机转换、`force` 分支、hook 执行顺序。
- 集成测试：模拟多个 worker（正常、卡死、拒绝停止），验证 CLI 输出和 server 日志；涵盖：
  - 正常关闭（未触发 force）。
  - 守护进程未运行（立即返回）。
  - Stop→Start 互斥：停机完成前拒绝新的 start。
  - 超时并强制终止时的清理。
