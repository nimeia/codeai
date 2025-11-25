# Index storage design（IR 优先）

以方案 B（分阶段 IR 规范化）为前提的索引存储设计，目标是：
- **幂等可重入**：IR、符号与向量分层存储，任何阶段失败可按游标恢复；
- **跨语言一致**：通过 IR schema 统一字段与命名，避免语言差异直接渗透到下游表结构；
- **增量友好**：watcher/变更队列只重写受影响的 IR 与符号行，同时维护版本号与 GC 标记；
- **可演进**：所有持久化对象带 schema 版本/格式号，便于重规范化或迁移。

## 工作目录布局
```
.code-nav/
├── metadata.db          # SQLite，持久化文件、符号、关系、索引游标
├── ir.cache/            # 可选：分片存储 IR JSON，便于断点续扫与调试
│   ├── v{schema}/       # 按 schema 版本隔离
│   └── {lang}/{hash}.json
├── hnsw.index           # 语义向量索引（可替换为 lancedb/sqlite-vector）
└── logs/                # 组件日志（indexer / watcher / embedder）
```

### 分支隔离与工作副本
- **首选做法**：为需要长期索引的分支创建独立的工作副本（如 `git worktree`），每个副本下拥有独立的 `.code-nav/` 目录与 `metadata.db`，避免跨分支写锁争用与数据互相覆盖。
- **单副本切换分支**：若在同一目录频繁切换分支，watcher 会捕获文件巨变并触发增量索引；但大改动可能导致 SQLite 重写范围大，推荐在切换后触发一次全量/增量刷新，或直接使用分支隔离的工作副本以保持写入串行且数据可追溯。
- **命名约定**：如需在同一物理路径下保留多套索引，可通过配置或启动参数指定独立 runtime 目录（例如 `.code-nav-main/`、`.code-nav-feature-x/`），并在 `project add` 时按目录注册，保证 worker 定向落盘。

## SQLite schema（示例）
- **files**：`id | path | lang | digest | size | mtime | version | is_deleted`
- **ir_blobs**：`id | file_id | lang | schema_ver | ir_hash | stored_path? | created_at`
- **symbols**：`id | file_id | ir_id | kind | name | fqname | visibility | span_start | span_end | doc | version | is_deleted`
- **relations**：`src_symbol_id | dst_symbol_id | kind (call/impl/use/import/extends)`
- **index_cursors**：`component | last_scanned_offset | schema_ver | updated_at`

> `ir_blobs.stored_path` 指向 `.code-nav/ir.cache/...` 以避免把大量 IR JSON 塞进 SQLite；小型项目可选择 inline（NULL 表示未落磁盘）。

## 向量存储
- **payload**：`vector_id -> symbol_id`，可选附加 `lang/module/fqname` 作为过滤标签；
- **版本标记**：为每批 embedding 记录 `batch_version`，与 `symbols.version` 对齐，便于无锁替换或延迟删除；
- **GC**：定期扫描 `symbols.is_deleted` / `version` 漂移的向量，执行软删除再压缩。

## 写入流程（全量 / 增量共用）
1) **解析 → IR**：按文件输出 IR，写入 `ir.cache` + `ir_blobs`，记录 `schema_ver` 与 `ir_hash`；
2) **IR → 符号**：从 IR 提取符号，幂等 upsert 到 `symbols`，对删除/重命名写 `is_deleted` 标记；
3) **符号 → 向量**：对需要 embedding 的符号批量生成向量，写入 `hnsw.index` 并记录 `batch_version`；
4) **游标落盘**：更新 `index_cursors`（语言扫描游标、watcher offset、embedding 批次号），保证可断点续扫。

## 并发与锁
- **单项目单 writer**：worker 负责串行化写入，避免 SQLite 锁竞争；
- **短事务**：IR/符号/关系分批事务写入，缩短锁持有时间；
- **崩溃恢复**：依赖 `version` + `ir_hash` 进行幂等比较，必要时重算 IR 并重写符号/向量。

## 元数据后端选择
- **默认继续使用 SQLite**：在“单项目单 writer + 分支隔离”前提下，SQLite 足以承载元数据写入与查询，便于分发与本地部署。
- **扩展空间**：向量库仍可替换为 lancedb/sqlite-vector；若未来出现千万级符号或高并发写需求，可在保持 IR → 符号流水线不变的前提下，引入可插拔的元数据后端（例如嵌入式 KV/关系引擎）。

## 观测与回报
- **指标**：每阶段记录文件数、IR 大小、符号/向量数量、耗时、失败样本；
- **状态回写**：将阶段状态写入 worker runtime 状态，供 `project status/list` 展示“索引中 / 向量生成中 / 完成”；
- **日志**：IR 解析、符号导出、向量写入分别打点，异常包含 `file_id + ir_hash` 方便重放。
