# `code-navd start` 第四步：控制端点建立

## 1. 端点类型与优先级
- 根据配置解析得到的 URI（`uds://`, `npipe://`, `tcp://`）决定监听方式：
  - **Unix/macOS**：默认 `uds://<runtime_dir>/master.sock`。
  - **Windows**：默认 `npipe://code-nav-master`。
  - **TCP**：用于回退或显式指定，例如 `tcp://127.0.0.1:7878`。
- 若配置未提供 URI，则按上述平台默认值生成。

## 2. 前置检查
- **UDS**：若 socket 文件已存在，尝试连接：
  - 连接成功 → 认定已有 master，退出。
  - 连接失败 → 删除旧文件，再创建新的 socket 文件。
- **Named Pipe**：调用 `CreateFile` 测试是否已有服务端；若存在则退出。
- **TCP**：在 `bind` 前检查端口占用；必要时支持 `SO_REUSEADDR` 以允许快速重启。

## 3. 监听器创建
- **UDS**：创建 `UnixListener`，设置文件权限为 `0o600`（仅当前用户可访问）；macOS 可额外开启 `SOCK_CLOEXEC`。
- **Named Pipe**：使用 `tokio::net::windows::named_pipe::ServerOptions` 设置单向/双向、缓冲大小、安全描述符（仅当前用户）。
- **TCP**：绑定到 loopback（默认 `127.0.0.1`），严禁默认绑定 0.0.0.0；可在配置开启 token 认证后再放宽。

## 4. 协议握手
- 使用协议 crate 扩展的 `MasterRequest`/`MasterResponse`：
  - 客户端连接后发送 `Hello { protocol_version, client_id, auth_token? }`。
  - master 校验版本/令牌不通过 → 返回 `Error { code: Unsupported/PermissionDenied }` 并关闭连接。
  - 校验通过 → 进入命令处理循环。

## 5. 处理模型
- 监听以 async runtime 方式运行：每当 `accept` 成功，spawn 新任务处理该连接。
- 每个连接支持多次请求（请求-响应式 JSON RPC）。
- 支持长连接命令（例如 `logs --follow`）时，连接任务可推送事件直到客户端断开。

## 6. 安全策略
- 默认仅允许本机访问（UDS + Named Pipe 本身具备此特性）。
- TCP 模式下：
  - 仅绑定 `127.0.0.1`，除非配置显式 `allow_remote=true`。
  - 支持 `auth_token`：请求必须携带 `Authorization` 字段或 JSON 内 `auth_token`；master 验证后继续。
  - 可选 IP allowlist。

## 7. 错误处理
- 监听器创建失败 → 立即退出 `start`，提示原因与修复方法（权限、端口冲突等）。
- 连接异常：
  - 单个连接出错时记录 WARN 日志，释放连接资源。
  - 不影响主监听器继续 `accept`。
- 若监听器在运行期间意外失效（例如 UDS 文件被删除），master 应记录错误并尝试重建或进入降级状态。

完成端点建立后，master 可以接收来自 CLI 的请求，进入 registry 恢复阶段。
