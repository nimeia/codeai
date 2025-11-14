# 后端服务功能概览

以下总结 code-nav 守护进程与 CLI 的核心职责，细化说明请参见 @/docs/features/backend-service/details.md。

1. 守护进程生命周期：start/stop/restart/status/logs，负责加载配置、管理监听、输出运行状态。
2. 索引管理：提供全量/增量索引、watcher 驱动的增量任务，确保 metadata 与向量库一致。
3. 语义搜索：自然语言查询 → embedding → ANN 检索，CLI `code-nav search` 对应。
4. Goto 导航：语义定位单个符号，支持编辑器跳转。
5. 结构化列表：类/方法/文件枚举，支持过滤分页。
6. 目录树：`code-nav tree` 提供目录结构视图。
7. 运行状态与信息：`status`/`info` 获取进度、版本、模型。
8. 配置与模型管理（扩展）：`config`、`models` 命令管理 embedding/后端选择。
