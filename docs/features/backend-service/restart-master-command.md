# `code-navd restart` 第一节：命令定位

## 1. 目标与价值
- 为 code-nav 提供标准化的“重启”能力，避免用户手动 stop→start，确保 master/worker 在受控流程下停机再启动。
- 主要用途：加载新配置或模型、清理长时间运行导致的资源问题、恢复异常状态、升级后重新生效。

## 2. 覆盖范围
1. **全局重启（master）**
   - `code-nav restart` 作用于守护进程：先停止控制端点并拒绝新 CLI 请求，然后向所有 worker 广播停机，最后 master 自身退出并由外层（CLI/systemd）自动拉起。
   - CLI 可通过 `--wait` 等参数等待 master 再次 Ready；若未指定，命令在 stop 阶段完成后即可退出。
2. **单项目 worker 重启**
   - `code-nav project restart --project <path|id>` 只影响一个 worker。
   - Master 暂停该项目的请求路由（返回“重启中”），先优雅停 worker，再复用 autostart/worker spawn 流程重启。
   - 可用于索引卡死、配置更改、模型失效等单项目场景。
3. **批量重启（扩展）**
   - 支持 `--project all` 或 `--failed`，按队列顺序或限流方式逐个 worker 重启，保障 registry 状态一致。

## 3. 与 start/stop 的关系
- restart = stop + start 的原子封装：
  - **Stop 阶段**：沿用 `code-nav stop` 的优雅停机机制（grace period、force、wait、幂等）。
  - **Start 阶段**：直接调用 master 的自动启动逻辑或 worker spawn 流程；对于 master，依赖 `code-navd start` 与运行环境（systemd、CLI 前台模式）配合。
- CLI 将两步组合成一个命令，减少窗口期与人为错误，server 负责保证状态一致。

## 4. 设计约束
- 控制端点可用性：重启单 worker 时，master 仍需服务其他项目；重启 master 时，需要与外部进程管理器协调，确保新的 master 及时接管控制端点。
- 幂等与状态提示：若目标已在重启或刚完成，应返回 `AlreadyRestarting` 或 `Completed`，允许脚本安全重试。
- 记录上下文：每次重启需在日志和 registry 中写入发起人、时间、是否强制、原因（可通过 CLI 参数传入），便于审计与诊断。
