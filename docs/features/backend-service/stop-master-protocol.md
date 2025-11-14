# `code-navd stop` 第二步：协议与路由

## 1. RPC 结构
- CLI 通过控制端点发送：
```json
{
  "id": "<uuid>",
  "method": "stop",
  "payload": {
    "force": false,
    "timeout_secs": 30
  }
}
```
- 新增 `Request::Stop(StopRequest)` 与 `Response::Stop(StopResponse)`：
  - `StopRequest { force: bool, timeout_secs: Option<u32> }`
  - `StopResponse { shutting_down: bool, message: Option<String>, state: StopState }`
  - `StopState` 取值：`Acked`, `AlreadyStopping`, `AlreadyStopped`.

## 2. 状态码与错误
- 正常响应：`Response::Stop`，`shutting_down=true` 表示 master 已进入停机流程。
- 错误响应：
  - `ErrorCode::Busy`：当前存在不可中断任务且未指定 `force`。
  - `ErrorCode::Unsupported`：旧版本不支持 stop。
  - `ErrorCode::PermissionDenied`：远程/多用户模式限制。
  - `ErrorCode::InternalError`：处理异常，CLI 应提示查看日志。

## 3. 路由与权限
- 控制端点监听器在握手阶段确认客户端是否具备 stop 权限：
  - 本地模式可通过 Unix socket `SO_PEERCRED`/Windows ACL 判定。
  - 可扩展 token/证书验证（`Authorization: Bearer ...`）。
- 若守护进程处于 `Stopping` 状态，新的 CLI 请求统一返回 `Response::Stop(AlreadyStopping)` 或 `ErrorCode::Busy`。

## 4. 进度反馈
- server 可在响应中附带提示信息（`message`），例如“等待 2 个 worker 退出”。
- 如需更详细的进度，可在 stop 请求后由 CLI 订阅 `status`，或 server 支持流式响应（可选扩展）。

## 5. 兼容性
- CLI 在发送 stop 前需检查 server 协议版本（通过 `info` 或握手）；若目标版本缺少 `Stop`，降级为直接发信号并提示用户升级。
- 协议字段保持向后兼容：未知字段在旧版本上安全忽略。

## 6. 测试
- 单元测试：序列化/反序列化 `StopRequest/Response`，模拟不同 `StopState` 组合。
- 集成测试：CLI 与 test-server 之间交换 stop 请求，验证 `force`、超时、错误路径。
