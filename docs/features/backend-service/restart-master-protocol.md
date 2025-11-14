# `code-navd restart` 第三节：协议扩展

## 1. RPC 类型
- 新增 `Request::Restart(RestartRequest)` 与 `Response::Restart(RestartResponse)`，沿用 Envelope 包装。
- `RestartRequest` 字段：
  - `scope: RestartScope`
    - `master`：重启守护进程。
    - `project { id: ProjectId }`：重启指定项目 worker。
    - （可选扩展）`projects { ids: Vec<ProjectId> }`：一次性重启多个 worker。
  - `force: bool`：允许 server 在优雅停机失败后执行强制终止。
  - `grace_secs: Option<u32>`：优雅停机宽限期；缺省使用 server 默认。
  - `wait_ready: bool`：server 是否等待目标 ready 后再返回。
  - `timeout_secs: Option<u32>`：server 端等待上限；到期视为失败。
  - `reason: Option<String>`：重启原因，写入日志/registry。

- `RestartResponse` 字段：
  - `state: RestartState`
    - `accepted`：请求已受理并开始执行。
    - `already_restarting`：目标正在重启。
    - `completed`：目标已重启并 ready（仅在 `wait_ready=true` 时返回）。
    - `failed`：重启失败或超时。
  - `message: Option<String>`：附加说明（例如剩余 worker 数、失败原因）。
  - `ready: Option<ReadyInfo>`（可选扩展）：包含新 PID、socket、时间戳等。

## 2. 错误码
- `ErrorCode::Busy`：存在不可中断任务，且未启用 `force`。
- `ErrorCode::NotIndexed`：指定项目不存在或未被 registry 管理。
- `ErrorCode::PermissionDenied`：无权限执行重启。
- `ErrorCode::Unsupported`：server 版本不支持 restart（CLI 可提示升级或降级为 stop+start）。
- `ErrorCode::InternalError`：执行过程中出现未捕获异常。

## 3. 状态协商与兼容
- CLI 在发送 restart 前可通过 `info` 或握手协商 server 功能版本；若不支持，CLI 提示用户升级并可选择 stop+start 回退。
- 所有新增字段需 `Option`/`#[serde(default)]`，保证旧客户端或 server 忽略未知字段。
- `RestartScope` 与 `RestartState` 使用 `snake_case` 序列化，与现有协议保持一致。

## 4. 批量结果（扩展）
- 若 `scope` 支持多个项目，可定义：
  ```rust
  pub struct RestartBatchResponse {
      pub results: Vec<ProjectRestartResult>,
  }
  pub struct ProjectRestartResult {
      pub project_id: String,
      pub state: RestartState,
      pub message: Option<String>,
  }
  ```
- CLI 根据每个项目的 `state` 决定输出与退出码；server 可在单次请求中返回所有结果。

## 5. 测试要点
- 序列化/反序列化：`RestartRequest`/`RestartResponse` 在各种 `scope`、`state`、`Option` 组合下保持稳定。
- 错误路径：模拟 `Busy`、`PermissionDenied`、`Unsupported` 等响应，确保 CLI 做出正确提示。
- `wait_ready` 行为：在测试 server 中验证 `wait_ready=true` 时阻塞至完成，`false` 时立即返回 `accepted`。
