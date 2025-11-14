# `code-navd start` 第一步：配置解析详解

## 1. 配置来源与优先级
1. **内置默认值**（编译时）：如 `runtime_dir = ~/.code-nav`, `socket = uds://~/.code-nav/master.sock`, `log_level = info`。
2. **全局配置文件**：`~/.config/code-nav/master.toml`（或 `master.json`，最终使用 serde 统一加载）。若文件不存在则跳过。
3. **环境变量**（以 `CODE_NAV_MASTER_` 为前缀）：支持覆盖关键字段，如 `CODE_NAV_MASTER_RUNTIME_DIR`, `CODE_NAV_MASTER_SOCKET`, `CODE_NAV_MASTER_LOG_LEVEL`。
4. **CLI 参数**：`code-navd start --config --socket --log-level --foreground --grace` 等，优先级最高。

合并策略：按上述顺序叠加，后者覆盖前者；解析完成后生成 `MasterConfig`。

## 2. `MasterConfig` 结构（建议放在 `server` crate）
```rust
pub struct MasterConfig {
    pub runtime_dir: PathBuf,
    pub registry_path: PathBuf,
    pub socket: SocketAddrKind,      // 枚举：Uds(PathBuf) | NamedPipe(String) | Tcp(SocketAddr)
    pub foreground: bool,
    pub log: LogConfig,
    pub grace_period: Duration,
    pub autostart: AutostartPolicy,  // None | All | List<Vec<ProjectId>>
    pub worker: WorkerDefaults,      // index_mode, watcher, env overrides 等
}
```

### 辅助结构
- `LogConfig { level: Level, file: Option<PathBuf>, rotate: Option<RotatePolicy> }`
- `AutostartPolicy`：
  - `None`：启动 master 时不自动启动任何项目。
  - `All`：恢复 registry 中所有“last_running=true”的项目。
  - `List(Vec<ProjectId>)`：仅启动列出的项目。
- `WorkerDefaults`：提供 `index_mode`, `watch`, `env`, `args` 等默认值，master 在 `project add` 时注入。

## 3. 控制端点与路径推导
- `runtime_dir` 默认 `~/.code-nav`；校验是否存在，不存在则 `create_dir_all` 并设置权限：
  - Unix：`0o700`；Windows：`Hidden + user-only ACL`。
- `registry_path` 默认 `runtime_dir/projects/registry.json`。
- `socket` 默认 `uds://<runtime_dir>/master.sock`；Windows 强制 `npipe://code-nav-master`，若用户指定 TCP（`tcp://127.0.0.1:7878`）需校验端口未占用。
- 所有路径在解析阶段展开 `~`、相对路径（基于当前工作目录）。

## 4. 校验规则
1. **路径可写**：`runtime_dir`/`registry_path` 所在目录必须存在且可写，否则报错停止。
2. **socket 冲突**：检测 UDS/Named Pipe 是否已存在，若存在尝试连接；连通表示已有 master，需提示退出；否则删除旧文件后重建。
3. **配置版本**：配置文件包含 `version` 字段，若低于当前 `CONFIG_VERSION`，尝试迁移；失败则输出升级指引。
4. **grace_period 合理**：必须在 `[1s, 300s]` 范围内，超出则回退到默认并输出警告。
5. **autostart 列表**：若指定项目路径需标准化并验证目录存在；不存在则记录 WARNING 但不阻塞 master 启动。

## 5. CLI 参数映射
- `--config <path>`：显式指定配置文件；若多次出现，以最后一个为准。
- `--socket <uri>`：覆盖配置中的 socket；支持 `uds://`, `npipe://`, `tcp://host:port`。
- `--log-level <lvl>`：映射到 `LogConfig.level`，合法值 `trace|debug|info|warn|error`。
- `--foreground`：布尔开关，覆盖 `MasterConfig.foreground`。
- `--grace <secs>`：转换为 `Duration`，写入 `MasterConfig.grace_period`。

参数解析应在 config 文件加载后执行，便于 CLI 覆盖文件内容。

## 6. 环境变量支持（示例）
| 变量 | 描述 |
| --- | --- |
| `CODE_NAV_MASTER_CONFIG` | 指定配置文件路径（优先于默认但低于 CLI）。 |
| `CODE_NAV_MASTER_RUNTIME_DIR` | 覆盖运行目录。 |
| `CODE_NAV_MASTER_SOCKET` | 覆盖控制端点。 |
| `CODE_NAV_MASTER_LOG_LEVEL` | 设置日志级别。 |
| `CODE_NAV_MASTER_GRACE` | 设置停机默认等待时间（秒）。 |

解析时可复用 `envy` 或自定义解析，统一前缀便于管理。

## 7. 错误处理与提示
- **缺失配置**：打印“未找到配置文件，使用默认值”并继续。
- **解析失败**：输出具体行号/字段，终止启动；CLI 返回非零码。
- **权限问题**：提供明确路径与建议（如 `chmod 700 ~/.code-nav`）。
- **socket 被占用**：输出检测到的现有进程信息，提示使用 `code-navd status` 或删除残留文件。

## 8. 示例流程（伪代码）
```rust
fn load_master_config(args: &CliArgs) -> Result<MasterConfig> {
    let defaults = MasterConfig::default();
    let file_path = args.config
        .or_else(|| env::var("CODE_NAV_MASTER_CONFIG").ok())
        .map(PathBuf::from)
        .unwrap_or(defaults.default_config_path());
    let file_cfg = read_config(&file_path).unwrap_or_default();
    let env_cfg = EnvConfig::from_env()?;        // 使用统一前缀解析
    let mut cfg = defaults.merge(file_cfg).merge(env_cfg).merge(args.into());
    cfg.normalize_paths()?;
    cfg.validate()?;      // 包含路径写权限、socket 冲突检查等
    Ok(cfg)
}
```

完成以上步骤后，将 `MasterConfig` 传递给后续“单实例检测”阶段使用。
