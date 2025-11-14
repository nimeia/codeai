# `code-navd restart` 第五节：状态与协作

## 1. AppState 扩展
- `state::AppState` 新增：
  ```rust
  pub struct MasterRestartState {
      pub status: RestartProgress,   // Idle | Restarting | Failed
      pub started_at: Option<DateTime<Utc>>,
      pub completed_at: Option<DateTime<Utc>>,
      pub reason: Option<String>,
      pub force: bool,
      pub initiator: Option<String>, // 例如 CLI 用户、PID、socket
  }
  pub struct ProjectRestartState {
      pub project_id: String,
      pub status: RestartProgress,
      pub started_at: Option<DateTime<Utc>>,
      pub completed_at: Option<DateTime<Utc>>,
      pub reason: Option<String>,
      pub force: bool,
      pub attempts: u32,
  }
  ```
- `RestartProgress = Idle | Restarting | Completed | Failed`，供 `status` 命令直接渲染。

## 2. Registry 字段
- `ProjectEntry` 增加：
  - `last_restart_at: Option<DateTime<Utc>>`
  - `last_restart_reason: Option<String>`
  - `last_restart_result: RestartResult`（Success/Failed/Timeout/Force）
  - `last_restart_duration_ms: Option<u64>`
  - `restart_failed_count: u32`
- 全局记录 `master_last_restart_at`, `master_last_restart_reason`, `master_last_restart_result`。

## 3. 自动启动与回收协作
- 重启流程复用 autostart 的 spawn 逻辑；在 `Restarting` 状态下暂停自动回收/自动重启任务，避免多个调度器竞争同一项目。
- 若 autostart 队列计划启动某项目而该项目正在重启，延迟启动直到 `Restarting` 结束。
- `restart` 触发的 worker 启动应标记来源（manual restart 与 autostart 区分），便于统计。

## 4. Watcher / Indexer 协作
- Stop 前通知 indexer/watcher checkpoint：
  - Flush 当前任务队列、提交 metadata/vectorstore 写入。
  - 记录最近处理的文件事件，以便重启后能检测遗漏。
- 重启后恢复：
  - watcher 重新订阅 FS 事件，如有 gap，通过 metadata diff 或全量校验补偿。
  - 长耗时任务（全量索引）可实现 pause/resume Hook，避免重启后重复工作。

## 5. 请求路由与反馈
- `status` 输出中展示 master/项目的 `Restarting` 状态、持续时间、剩余 worker（对于 master）。
- 对处于 `Restarting` 的项目，CLI 搜索/索引请求返回 `ErrorCode::Busy`，`message="project restarting (started_at=...)"`。
- Master 重启期间，控制端点在未 ready 前统一返回“master restarting”，指引用户稍后重试。

## 6. Metrics 与告警
- Metrics：
  - `restart_total{scope}`、`restart_fail_total{scope}`
  - `restart_duration_seconds{scope}`
  - `project_restart_in_progress`
  - `restart_force_total`
- 告警触发条件：
  - 某项目连续重启失败超过阈值（触发人值/日志）。
  - Master 重启耗时超过 SLA。
  - Restart 状态期间 CLI 请求拒绝率过高。
