# `code-navd start` 第五步：项目 registry 恢复

## 1. 存储结构
- 默认使用 JSON 文件 `runtime_dir/projects/registry.json`（可替换为 SQLite，结构一致）。
- 文件结构示例：
```json
{
  "version": 1,
  "projects": [
    {
      "project_root": "/path/to/repo",
      "project_id": "hash-of-path",
      "worker_socket": "uds:///Users/alice/.code-nav/projects/abc.sock",
      "last_running": true,
      "last_state": "Running",
      "last_updated": "2024-05-01T08:12:34Z",
      "metadata": {
        "index_version": "2024-04-30T09:00:00Z",
        "model": "ggml-small"
      }
    }
  ]
}
```
- `project_id` 推荐采用 `sha1(project_root)` 或 UUID（确保跨重启稳定）。

## 2. 加载流程
1. 检查 `runtime_dir/projects/` 目录，不存在则创建。
2. 尝试读取 `registry.json`：
   - 不存在 → 创建空结构并写入。
   - 存在 → 解析 JSON，校验 `version`。
3. 若版本落后，执行迁移脚本（例如补充 `project_id` 字段）。
4. 将结果加载到内存结构 `ProjectRegistry`（`HashMap<ProjectId, ProjectEntry>`），并存储标准化 `project_root`（绝对路径 + 符号链接解析）。

## 3. 状态同步
对每个项目执行：
1. 检查 `project_root` 是否仍存在；如不存在，标记 `Orphan`。
2. 若 `worker_socket` 存在：
   - 尝试连接/Ping；成功 → 更新状态为 `Running`，可请求 worker 上报 PID/索引信息。
   - 失败 → 标记 `Stopped`，清理残留 socket 文件。
3. 若有项目级锁/ PID 文件存在，验证与实际 worker 是否匹配，不匹配则清理。

## 4. 清理与更新
- 对 `last_state="Running"` 但实际已停止的项目，更新为 `Crashed` 或 `Stopped` 并记录 `last_updated`。
- 删除 registry 中重复或无效条目（如 `project_root` 重复）。
- 更新完成后写回文件：写入临时文件 `registry.json.tmp`，`fsync`，再 `rename` 覆盖原文件。

## 5. 自动启动决策
- 根据 `MasterConfig.autostart` 策略：
  - `None`：仅记录状态，不自动启动。
  - `All`：对 `last_running=true` 的项目排队启动。
  - `List`：仅启动列出的项目 ID/路径。
- master 将结果记录到待启动队列，下一步执行 spawn。

## 6. 并发访问
- 运行期间，所有对 registry 的读写通过内存结构，使用 `RwLock` 保护。
- CLI `project list/status` 直接读取内存数据。
- 每当 worker 状态变化（启动/停止/崩溃），master 更新内存并触发一次写回（可做节流，例如 1s 内合并）。

## 7. 错误处理
- **文件损坏**：无法解析时，将原文件备份为 `registry.json.bak.<timestamp>`，创建新空文件，并记录 ERROR。
- **权限不足**：终止启动并提示“无法写入 registry，检查 runtime_dir 权限”。
- **磁盘满**：写回失败时，保持内存状态但 WARN，提示用户释放空间。

完成 registry 恢复后，master 拥有当前所有项目的状态，为自动启动和 CLI 查询提供基础。
