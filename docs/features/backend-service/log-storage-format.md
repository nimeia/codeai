# 日志存储格式规范

本文描述 master 与 worker 日志的统一存储格式、落盘规则和缓冲策略，确保日志可在不同介质间一致传输、易于解析，并满足自定义路径与带宽受控的需求。

## 1. 设计目标
- **格式统一**：落盘、暂存（spool）与网络传输均使用同一事件结构，便于跨介质回放与转储。
- **轻量可靠**：采用换行分隔的 JSON（NDJSON），单行自包含，避免跨行解析成本；字段固定，利于压缩与下游分析。
- **路径可配置**：master 日志文件与 worker spool 目录由配置/CLI 指定，可放置在独立分区以避免主目录膨胀。
- **限流友好**：worker 先写入 spool（本地落盘、可轮转），由 supervisor 批量读取并按窗口大小推送给 master，防止突发流量撑爆网络。

## 2. 事件结构（JSON Schema）
每条日志均为一行 JSON，对应 `LogEvent` 语义，示例：

```json
{"ts":1715501730,"level":"info","source":"worker:calc-service","target":"watcher","message":"rescan finished","fields":{"files":128,"duration_ms":432}}
```

字段约定：
- `ts`：Unix 时间戳（秒），UTC；由产生事件的进程写入。master 侧的 tracing 捕获层也会在写入前强制刷新该字段，确保落盘与回放统一。
- `level`：`trace|debug|info|warn|error`。
- `source`：`master` 或 `worker:<project_id>`。
- `target`：可选组件名（如 `watcher`、`indexer`、`rpc`）。
- `message`：主消息字符串。
- `fields`：键值字段（字符串/数字/bool 自动序列化为 JSON），不可嵌套对象。

## 3. Master 落盘格式
- **文件编码**：UTF-8 NDJSON，按上述字段写入；不带 ANSI 颜色。
- **时间格式**：`ts` 为 Unix 秒级时间戳，与 RPC 传输保持一致，便于快速比较与过滤。
- **默认位置**：当后台运行或配置显式指定时写入 `log.file` 路径；路径可为绝对或相对于 `runtime_dir` 的相对路径。
- **轮转**：建议按大小轮转（例如 32 MB、保留 5 份），命名 `master.log`, `master.log.1`, ...；超出保留数的旧文件删除。

## 4. Worker Spool（暂存）格式
- **目录结构**：`<worker_log_spool>/<project_id>/`；每个项目独立子目录，避免跨项目串写。
- **文件命名**：按时间或序号切分，例如 `00001.log`, `00002.log`；单文件大小（默认 8 MB）或时间窗口（如 1 分钟）达到阈值即滚动。
- **写入格式**：与 master 相同的 NDJSON 行，字段完全一致；写入时即可落盘便于断点续传。
- **上传节奏**：supervisor 定期批量读取 spool 文件并按配置的批大小/速率发送到 master，发送成功后删除或打标已传文件。
- **异常恢复**：若网络中断，spool 中的文件原样保留；恢复后继续按顺序推送，保证日志不丢失且不重复。

## 5. 回放与压缩
- **历史回放**：当前阶段回放来源仅限于内存 ring buffer；后续若需要补齐更早的历史，可直接从 master log 文件尾部回溯，因格式一致，扩展成本低。
- **压缩存档**：后台可选定期将过期的 spool/master 日志归档为 `.gz`，压缩前后均保持 NDJSON 结构，方便后续分析/导入。

## 6. 兼容与迁移
- **向前兼容**：旧的纯文本日志可继续读取但不再作为持久化格式输出；必要时可提供小工具将文本转为 NDJSON。
- **工具链支持**：下游可直接使用 `jq`/`rg`/`python` 等处理单行 JSON；E2E 测试应校验新格式可被 `code-nav logs --json` 原样消费。
