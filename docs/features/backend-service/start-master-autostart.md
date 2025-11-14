# `code-navd start` 第六步：自动启动策略

## 1. 策略类型
- `None`：master 启动时不自动启动任何项目。
- `All`：对 registry 中 `last_running=true` 的项目全部启动。
- `List`：仅启动配置中列出的项目（可用绝对路径或 `project_id`）。
- `OnDemand`（可选扩展）：仅当 CLI 请求目标项目时才启动，对冷项目友好。

策略通过 `MasterConfig.autostart` 指定，默认 `All`。

## 2. 启动队列构建
1. 遍历 registry：
   - 若策略为 `All` → `last_running=true` 的项目加入队列。
   - 若策略为 `List` → 匹配 `project_root` 或 `project_id` 的项目加入队列。
   - 若策略为 `OnDemand` → 不立即加入，等待 CLI 请求。
2. 队列元素包含：`project_id`, `project_root`, `worker_socket`, `worker_config`（由 `worker_defaults` 合并项目自定义配置）。
3. 记录队列状态，便于 `status` 输出（例如“queued”、“starting”、“running”）。

## 3. 启动执行
- Master 控制 `max_concurrent_starts`（配置项，例如 2）防止同时 spawn 太多 worker。
- 启动命令示例：
  ```
  code-navd --role worker \
            --project /path/to/repo \
            --socket uds:///Users/alice/.code-nav/projects/<id>.sock \
            --index-mode auto --watch
  ```
- 每次启动后等待 worker ready：
  - 通过 ping worker socket 或等待 worker 主动发送 Ready 消息。
  - 设置 `startup_timeout`（如 30s），超时视为失败。
- 成功：更新 registry `last_state="Running"`, `last_running=true`, `worker_pid=...`。

## 4. 失败与重试
- 启动失败原因（退出码、超时）记录在 registry，并输出 WARN。
- 配置 `retry_count` 和 `retry_backoff`（如指数退避），允许 master 自动重试 N 次；超过次数标记 `last_state="Failed"`。
- 若项目路径不存在/权限不足，直接标记为 `Orphan` / `PermissionDenied`，不再重试。

## 5. 空闲回收（可选）
- `auto_shutdown_idle=true` + `idle_timeout`（例如 30 分钟）：
  - 若 worker 在超时时间内没有收到请求且没有索引任务，master 发送 `Shutdown` 停止 worker。
  - 停止后保持 `last_running=false`，下次 CLI 请求时再根据策略启动。
- CLI `project add --autostart` 可将项目加入 `List` 策略。

## 6. 监控与反馈
- 将自动启动队列状态暴露给 `code-navd status`：
  ```
  Project foo: queued (autostart)
  Project bar: starting (attempt 1/3)
  Project baz: failed (permission denied)
  ```
- Metrics 示例：`autostart_queue_length`, `worker_start_total`, `worker_start_failed_total`.

完成自动启动策略后，进入主循环与信号处理阶段。
