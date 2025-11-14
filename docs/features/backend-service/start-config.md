# `code-navd start` 配置解析与环境校验

1. **配置来源与优先级**
   - 默认配置：`~/.config/code-nav/config.toml`（全局）或 `<project>/.code-nav/config.json`（项目内）。
   - 环境变量：例如 `CODE_NAV_PROJECT_ROOT`、`CODE_NAV_SOCKET`、`CODE_NAV_MODEL`。
   - CLI 参数：`code-navd start --project <path> --socket uds://...`，优先级最高。
   - 解析顺序：默认 → 环境变量 → CLI，合并后生成 `ServerConfig`。

2. **关键配置项**
   - `project_root`：必需；若未显式提供则默认当前工作目录。
   - `runtime_dir`：默认为 `<project>/.code-nav/`；若不存在则创建，权限 0700。
   - `listener`：UDS/Named Pipe/TCP 端点，格式如 `uds://~/.code-nav/code-navd.sock` 或 `tcp://127.0.0.1:7878`。
   - `model`：嵌入模型或远程 API 选择（本地文件路径、远程 endpoint、token）。
   - `index`：索引策略，例如并发度、增量间隔、watcher 开关。
   - `logging`：日志级别、输出位置、轮转策略。

3. **校验流程**
   - 路径合法性：`project_root` 必须存在可读，`runtime_dir` 需可写；若缺失则创建 `.code-nav/`。
   - 监听端点：校验 URI scheme、端口是否被占用（TCP）、UDS 路径长度/权限、Windows Named Pipe 名称规范。
   - 模型/依赖：本地模型文件存在、远程模式下 API key 已设置。
   - 配置版本：比较 `config_version`，若旧版本则执行迁移或报错。

4. **失败策略**
   - 严重错误（路径不可写、端点冲突、配置语法错误）直接退出并返回非零码，日志中给出修复建议。
   - 可选项缺失使用默认值并记录 WARN。
   - `code-navd start --check-config` 只执行上述解析与校验，成功返回 0，失败返回错误描述。

此步骤完成后得到的 `ServerConfig` 将作为后续单实例检查、资源加载等流程的输入。
