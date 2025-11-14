# `code-navd stop` 第一步：命令入口与交互

## 1. 使用场景
- 关闭守护进程以释放系统资源、升级版本、切换配置或在同一项目上重新索引。
- 自动化脚本/服务管理器（systemd、launchd）在停服务前统一调用。
- 幂等：目标进程不存在或正在停机时，命令返回成功并提示当前状态。

## 2. 命令语法
```
code-nav stop
    [--grace <seconds>]
    [--timeout <seconds>]
    [--force]
    [--yes]
```
- `--grace`：通知 worker 后等待优雅退出的时间，默认 10s。
- `--timeout`：CLI 等待 server 返回最终结果的上限（默认 30s），超过即退出并提示仍在停止中。
- `--force`：当 server 报告无法在宽限期内停止时，允许其升级为强制终止（`SIGKILL`/`TerminateProcess`）。
- `--yes`：跳过交互式确认提示，便于脚本使用。

## 3. 用户交互与输出
1. CLI 检查控制 socket/PID 是否存在；若未运行打印“code-navd 未在运行，跳过”并返回 0。
2. 正常路径输出：
   - “发送停止请求...”
   - “等待守护进程退出（剩余 n 秒）...”
   - “停止成功，PID xxx 已退出。”
3. 异常提示：
   - “守护进程拒绝停止：Busy (正在执行全量索引)”
   - “等待超时，可重试或使用 --force”
4. 退出码：
   - `0`：成功或目标本就未运行。
   - `2`：server 拒绝（Busy/Unsupported/PermissionDenied）。
   - `3`：等待超时（server 可能仍在停止中）。
   - `1`：其它 CLI 侧错误（连接失败、参数非法等）。

## 4. 依赖关系
- CLI 必须能够定位控制端点（Unix Domain Socket、命名管道、TCP）；与 `start` 命令共用 `runtime_dir` 解析逻辑。
- 若控制端点不可用但 PID 文件存在，可退化为发送系统信号（`SIGTERM`）但仍建议走 RPC，以便 server 完成清理。

## 5. 验证点
- 子命令解析覆盖参数组合（含默认值）。
- 幂等性测试：重复执行 stop，不报错。
- 交互式确认（默认提示“确定要停止 code-navd? [y/N]”），非交互环境自动拒绝除非提供 `--yes`。
