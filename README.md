# code-nav

Rust 本地代码导航与搜索工具的初始骨架。当前仓库仅包含基础 workspace、crate 目录以及占位实现，便于后续迭代。

## 工作区结构

- `crates/core`：索引、搜索、embedding 等核心逻辑。
- `crates/server`：守护进程入口（code-navd）。
- `crates/cli`：命令行客户端。
- `crates/protocol`：RPC 数据结构与错误定义。
- `scripts/`：后续构建/运维脚本。
- `.code-nav/`：运行期数据目录（当前放置占位 `.gitkeep`）。

## 开始构建

```bash
cargo fmt
cargo build
```

构建完成后会在 `target/debug/code-nav`（或 release 模式对应路径）下生成 CLI 可执行文件。可以通过以下命令查看已经接入的 `project` 子命令：

```bash
cargo run -p code-nav-cli -- project --help
```

示例输出：

```
Usage: code-nav project <COMMAND>

Commands:
  add      Register a project and schedule worker initialization
  remove   Remove a project from the registry
  list     List registered projects and their runtime state
  status   Show detailed runtime information for a single project
  restart  Restart one or multiple project workers
  help     Print this message or the help of the given subcommand(s)
```

同样地，`code-nav logs` 也已经提供了参数解析与预览能力，可通过以下方式查看：

```bash
cargo run -p code-nav-cli -- logs --help
```

日志命令支持 `--target/--project/--since/--limit/--follow/--level/--json/--color` 等参数，并会把解析结果整理成 JSON 结构，便于在后续实现守护进程日志流接口时直接复用。

后续可在各 crate 中逐步替换占位实现，接入 tree-sitter、embedding 后端、HNSW 向量库等真实逻辑。
