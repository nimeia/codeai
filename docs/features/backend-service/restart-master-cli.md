# `code-navd restart` 第二节：CLI 设计

## 1. 命令结构
```bash
# 全局：重启 master
code-nav restart
    [--wait | --no-wait]
    [--grace <seconds>]
    [--force]
    [--timeout <seconds>]
    [--reason <text>]
    [--yes]

# 单项目/批量：重启 worker
code-nav project restart
    --project <path|id> [--project <path|id> ...]
    [--wait | --no-wait]
    [--grace <seconds>]
    [--force]
    [--timeout <seconds>]
    [--reason <text>]
    [--yes]
    [--all]        # 可选扩展：对 registry 中所有项目执行
    [--failed]     # 可选扩展：仅对状态 Failed/Crashed 项目执行
```

## 2. 参数语义
- `--project <path|id>`：指定要重启的项目；在 `code-nav restart` 下使用时等价于项目级重启，未指定则作用于 master。
- `--wait/--no-wait`：默认 `--wait`，CLI 阻塞直到 server 报告重启完成或达到 `--timeout`；`--no-wait` 发送请求后立即返回。
- `--grace <seconds>`：stop 阶段的优雅停机宽限期（默认 10s，与 stop 命令一致）。
- `--force`：优雅停机失败后允许 server 触发强制终止（默认为关闭）。
- `--timeout <seconds>`：CLI 等待整个重启流程的时间上限（默认 60s）；到期仍未 ready -> 退出码 3。
- `--reason <text>`：附带人类可读原因，写入日志/registry 便于审计。
- `--yes`：跳过确认提示，脚本化使用。
- `--all`/`--failed`（扩展）：在 `project restart` 中指定批量目标；可与 `--project` 组合（先显式列表，再补充匹配条件）。

## 3. 交互与输出
1. **确认提示**：在重启 master 或批量项目时默认提示“确定要重启 code-navd/这些项目？[y/N]”；非交互环境需显式传 `--yes`。
2. **阶段输出**（`--wait` 时）：
   - `发送重启请求 (scope=master|project, grace, force, reason)`
   - `停止中...（剩余 n 秒）`
   - `启动中...等待 ready`
   - `完成：master 重启成功` / `项目 foo 重启成功`
3. **错误输出**：
   - Server 拒绝：打印 `拒绝：Busy (全量索引进行中)` 等详细信息。
   - 超时：提示“仍在重启中，可稍后运行 status/inspect 日志”。
   - CLI 连接失败：打印控制端点位置、建议运行 `code-nav status`。

## 4. 批量与幂等行为
- 支持多个 `--project`，默认顺序执行；可扩展 `--max-parallel` 控制并发度。
- 对已处于 `Restarting` 或 `Completed` 状态的项目，输出对应提示但不视为错误。
- 总体 exit code：
  - `0`：所有目标成功完成或已在期望状态。
  - `2`：至少一个目标被拒绝（Busy / PermissionDenied / NotIndexed）。
  - `3`：等待超时（仍在重启中）。
  - `1`：CLI 自身错误（参数、I/O、连接失败）。
