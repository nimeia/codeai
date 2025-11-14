# `code-navd start` 第八步：启动反馈

## 1. 成功反馈
- **前台模式**（`--foreground`）：在 stdout 打印提示，例如：
  ```
  code-navd master running
    pid: 12345
    socket: uds:///Users/alice/.code-nav/master.sock
    projects running: 3
  ```
  并持续输出日志。
- **后台模式**：`code-navd start` 命令完成后打印简要信息（PID、socket），同时写入 `<runtime_dir>/master.ready` 作为 readiness 标记。
- 记录日志事件 `master_start { pid, socket, projects_running }`，并推送到 metrics `master_startup_duration_seconds`.

## 2. Ready / Wait 选项
- 支持 `--wait-ready`（默认开启）：start 命令在控制端点可接收请求后才返回 0；期间轮询端点或等待内部信号。
- 超时（默认 30s）时返回非零，提示用户查看日志或执行 `code-navd check`。
- 若 `--no-wait`，命令在启动流程进入后台后立即返回，适用于系统服务脚本。

## 3. 后续指引
- 成功后在 stdout/log 中提示：
  - “Use `code-navd status` to view managed projects.”
  - “Use `code-navd project add --project <path>` to add a new project.”
- 失败时指引运行 `code-navd check` 或查看日志路径。

## 4. 失败清理
- 任意阶段失败需确保：
  - 释放/删除 `master.lock`、`master.pid`。
  - 删除已创建但未使用的 socket/pipe。
  - 写入 error 日志并返回非零 exit code。

## 5. 系统集成
- 提供 `--systemd-notify` 选项：成功启动后向 systemd 发送 `READY=1`。
- Windows 可选写入 Event Log，便于系统管理员查看。

完成启动反馈后，`code-navd start` 全流程设计完结。
