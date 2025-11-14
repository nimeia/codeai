# `code-navd start` 第三步：日志与监控初始化

目标：在 master 启动早期建立稳定的日志与可选监控输出，便于调试和生产环境追踪。

## 1. 日志配置来源
- 使用 `MasterConfig.log`（见配置解析文档）决定：
  - 日志级别 `trace|debug|info|warn|error`
  - 输出目标：STDOUT（前台模式默认）、文件（后台模式默认）
  - 轮转策略（按大小或按天）
  - 格式（文本/JSON）、时间戳格式（UTC/本地）

## 2. 初始化顺序
1. 在成功获取单实例锁后立即初始化日志，这样后续步骤的事件都会被记录。
2. 若 log 文件路径在 `runtime_dir/logs` 下，确保目录存在并具备写权限。
3. 初始化 tracing subscriber（例如 `tracing_subscriber::fmt` + rolling appender）。
4. 若配置启用 JSON 输出，则使用 `tracing_subscriber::fmt().json()`。

## 3. 文件轮转策略
- 推荐复用 `tracing-appender`：
  - `RollingFileAppender::new(Rotation::NEVER | HOURLY | DAILY, dir, file)`
  - 或自定义按大小轮转（`size_mb`, `keep`）：
    - 检测文件超过阈值时重命名为 `master.log.<timestamp>`，最多保留 `keep` 份。
- Windows 路径注意使用 `\\` 或 `PathBuf`。

## 4. STDOUT/后台模式切换
- `foreground=true`：默认输出到 STDOUT/stderr，方便调试。
- `foreground=false`：默认写文件；如未配置 `file`，自动创建 `runtime_dir/logs/master.log`。
- 支持 `--log-file -` 强制 STDOUT。

## 5. 日志上下文
- 在 subscriber 中设置全局字段（`master_id`, `socket`, `pid`）。
- 对 CLI 请求、worker 启停、错误等场景记录结构化字段（project_root, worker_pid, request_id）。

## 6. 监控 / Metrics（可选）
- 若配置启用，启动一个 metrics 采集器，常见方案：
  - `prometheus` exporter 监听 `127.0.0.1:<port>`。
  - 或者把指标写入本地文件供外部采集。
- 监控项示例：`master_up`, `worker_count`, `requests_total`, `worker_restart_total`, `index_jobs_in_queue`。
- 若不开启，则 skip。

## 7. 健康检查端点（可选）
- 基于 HTTP（如 `http://127.0.0.1:port/healthz`）或 Unix socket，返回 `{"status":"ok","workers":num}`。
- 用于系统守护进程监控（launchctl/systemd）。

## 8. 错误处理
- 日志初始化失败（例如路径不可写）：
  - 若 STDOUT 可用，打印错误并退出。
  - 提示用户检查权限或 `log.file` 配置。
- Metrics/健康检查启动失败：
  - 记录 WARNING，但不阻止 master 启动，除非配置显式 `required=true`。

完成日志与监控初始化后，进入控制端点创建步骤。
