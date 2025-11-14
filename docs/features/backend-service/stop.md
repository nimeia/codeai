# `code-navd stop` 功能设计

1. **目标**
   - 优雅终止运行中的守护进程，确保索引任务、数据库写入、watcher 等资源安全释放。

2. **流程**
   - *定位实例*：通过 PID/lock 文件或控制 socket 判断进程是否存在；若未运行则提示并返回成功（保持幂等）。
   - *发送停止信号*：
     - 控制接口存在时，发送 `ControlRequest::Shutdown`（UDS/Named Pipe/TCP 管理端口）。
     - 仅有 PID 时，Unix 使用 `SIGTERM`，Windows 使用 `GenerateConsoleCtrlEvent` 或自定义 pipe。
   - *等待退出*：默认等待 `grace_period`（可由 `--timeout`/`--grace` 指定），轮询 PID/lock 或监听控制响应，检测进程退出即返回。
   - *超时处理*：若超时未退出，根据 `--force` 决定是否发送 `SIGKILL`/`TerminateProcess`；否则报超时并提示查看日志。
   - *清理*：移除 PID/lock 文件，更新状态缓存。

3. **CLI 选项与反馈**
   - `code-navd stop [--grace 10] [--force]`。
   - 输出阶段性提示（“发送停止请求”“等待进程退出”“已停止/超时”），并指引查看日志位置。
