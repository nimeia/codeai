# `code-navd start` 第二步：单实例检测

目标：保证任意时刻仅有一个 master 进程运行，避免端口/资源冲突。流程涵盖锁文件、PID 文件、僵尸检测与错误反馈。

## 1. 锁与 PID 文件位置
- 使用 `runtime_dir/master.lock` 作为互斥锁文件。
- 使用 `runtime_dir/master.pid` 存储当前 master 信息。
- 若 `runtime_dir` 不存在，由配置解析阶段创建好；此处仅关注锁逻辑。

## 2. 锁实现
- **Unix/macOS**：打开 `master.lock`（`O_CREAT | O_RDWR`），使用 `fcntl`/`flock` 获取独占锁。
- **Windows**：使用 `CreateFile` + `LockFileEx`（或命名互斥体 `CreateMutex`），与 Unix 逻辑一致。
- 锁获取策略：
  1. 尝试获取锁；成功 → 继续写 PID。
  2. 失败 → 读取 `master.pid`，检查 PID 是否仍存活；若存活 → 认为已有实例运行。
  3. 若 PID 不存在（僵尸锁）→ 输出 WARNING，清理 `master.lock/master.pid` 后重试。

## 3. PID 文件内容
```text
pid=12345
started_at=2024-05-01T08:12:34Z
socket=uds:///Users/alice/.code-nav/master.sock
host=hostname
version=0.1.0
```
- 可使用 JSON/TOML，便于其它命令读取。
- `started_at` 采用 UTC ISO8601，便于日志关联。

## 4. 运行中检测
- **Unix**：`kill(pid, 0)`；返回 0 表示存活，`ESRCH` 表示不存在。
- **Windows**：使用 `OpenProcess` + `GetExitCodeProcess` 判断。
- 若检测到存活：
  - 输出提示：“code-navd 已在 PID xxx 运行（启动于 ...，socket ...）”
  - CLI 返回成功或特定 exit code（如 `EEXIST`），提醒用户执行 `code-navd status` 或 `code-navd stop`。

## 5. 僵尸锁处理
- 条件：锁文件存在但 PID 不存活。
- 流程：
  1. 记录 WARNING 日志（包含旧 PID 与 `started_at`）。
  2. 删除 `master.lock`、`master.pid`。
  3. 重新尝试加锁（最多 N 次，避免无限循环）。

## 6. PID 写入
- 成功获取锁后，写入 `master.pid`：
  - 包含 PID、启动时间、socket、版本、配置路径。
  - 写入后 `fsync` 确保落盘。
- 若写入失败（磁盘满等）：
  - 释放锁并报错，阻止启动。

## 7. 释放锁
- 正常退出：销毁监听前先删除 `master.pid`，再释放/删除 `master.lock`。
- 异常退出：锁会随进程释放，但文件仍在；下次启动检测僵尸时处理。

## 8. CLI/日志反馈
- 成功：在日志或 STDOUT 中打印“锁已获取，PID 写入 ...”。
- 已有实例：给出 PID、socket、启动时间，并指引 `code-navd status`/`stop`。
- 僵尸：打印清理信息，提醒用户检查潜在崩溃原因。

此步骤完成后即可进入日志/监控初始化。
