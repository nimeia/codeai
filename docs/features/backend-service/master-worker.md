# code-navd Master/Worker 架构设计

## 1. 总体结构
- `code-navd` 承担 **master** 角色，常驻系统，负责所有 CLI 请求入口与 worker 生命周期管理。
- 每个项目由独立 **worker 进程** 负责索引/搜索/监听等任务（复用 server crate 逻辑，可命名为 `code-nav-worker`）。
- CLI 默认连接 master，由 master 根据 `project_root` 路由请求至对应 worker；必要时 CLI 可通过 `--direct` 直连 worker（调试用途）。

## 2. master 职责
- **配置与控制端点**：启动时加载 `~/.config/code-nav/master.toml` 等配置，创建统一控制端点：
  - Linux/macOS：`~/.code-nav/master.sock`（Unix Domain Socket）。
  - Windows：`\\.\pipe\\code-nav-master`（Named Pipe）。
- **项目 registry**：维护 `project_root`、worker PID、IPC 地址、索引状态、最近活动时间等信息。
- **请求路由**：接收 CLI 的 `project start/stop/status` 与 `search/goto/list/tree` 等命令：
  - 若 worker 未运行 → 启动新 worker（spawn/fork），等待 ready（探测 worker IPC）。
  - 将请求转发到 worker 并返回结果。
- **监控与调度**：监听 worker 进程状态（waitpid/WaitForSingleObject），记录崩溃并可选自动重启；根据配置限制并发 worker 数、回收闲置 worker。
- **优雅停机**：master 停止时向所有 worker 发送 `Shutdown`，等待退出后关闭控制端点。

## 3. worker 职责
- 启动参数示例：`code-nav-worker --project <path> --socket uds://...`，socket 由 master 分配（UDS/Named Pipe/TCP loopback）。
- 执行单项目的完整功能：索引构建、watcher、语义/结构化搜索、goto、状态/信息查询。
- 使用 protocol crate 定义的 RPC 与 master 通信，master 充当代理。
- 退出前通知 master 更新 registry；异常退出由 master 监控线程捕获。

## 4. CLI 流程
- `code-nav project start <root>`：CLI → master（启动指定项目 worker）。
- `code-nav project stop <root>`：CLI → master（停止 worker）。
- `code-nav search --project <root> "query"`：CLI → master → worker → master → CLI。
- CLI 可缓存最近使用的 worker IPC，以在 `--direct` 模式下绕过 master（例如脚本自动化场景）。

## 5. IPC 与协议
- master 控制端与 worker IPC 均使用统一协议（可复用 `code-nav-protocol`，扩展 master 专用方法，例如 `MasterRequest::{ProjectStart, ProjectStop, Route}`）。
- IPC transport 抽象：提供统一 trait，内部根据平台选择 UDS 或 Named Pipe；TCP loopback 作为回退方案。

## 6. 生命周期管理
- master 启动：
  1. 初始化日志/配置，创建控制端点。
  2. 扫描 `~/.code-nav/projects/*`，尝试对接已有 worker；失联则清理 registry。
- master 运行：持续监听 CLI 请求、监控 worker 状态、按需调度。
- master 停止：发出 `Shutdown` 给所有 worker，等待退出后再退出自身。
- worker 启停：由 master 管控；worker 自身的 `start` 流程沿用之前设计（配置解析、单实例锁、资源加载等）。

该架构在 Linux/macOS/Windows 上均可实现，关键在于抽象跨平台 IPC 与子进程管理；master 增加统一入口避免端口爆炸，同时提供项目级隔离与调度能力。

## 7. code-navd 命令行
- `code-navd start`：启动 master（读取配置、创建控制端点并守护运行）；若已运行则返回状态。参数：`--config <path>`、`--socket <uri>`、`--foreground`、`--log-level <lvl>`、`--grace <secs>`。
- `code-navd stop`：请求 master 优雅退出，必要时 `--force` 强制终止所有 worker。参数：`--socket <uri>`、`--grace <secs>`、`--force`。
- `code-navd status`：查看 master 及所有 worker 的总体状态（PID、索引版本、最近活动）。参数：`--socket <uri>`。
- `code-navd project add --project <path>`：将项目纳入管理，创建 `.code-nav/`、写入 registry，并自动启动该项目 worker。可选 `--socket <uri>`、`--index-mode <full|incremental|auto>`、`--watch`。
- `code-navd project remove --project <path>`：停止并移除项目 worker，清理 lock/PID，registry 删除记录。可选 `--socket <uri>`、`--grace <secs>`、`--force`。
- `code-navd project list`：列出所有受管项目与 worker 状态（运行/停止/异常）。参数：`--socket <uri>`。
- `code-navd project status --project <path>`：查看指定项目的详细状态（索引进度、任务队列、watcher、监听端点）。参数：`--project <path>`、`--socket <uri>`。
- `code-navd project restart --project <path>`：对指定项目执行 stop→start，常用于重载配置/模型。共享 `--socket <uri>`、`--grace <secs>`、`--index-mode ...`。
- `code-navd check [--project <path>]`：执行配置/环境自检；若指定项目，则检查其 `.code-nav/`、权限与端点可用性。参数：`--config <path>`、`--socket <uri>`、`--project <path>`。
