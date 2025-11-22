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

后续可在各 crate 中逐步替换占位实现，接入 tree-sitter、embedding 后端、HNSW 向量库等真实逻辑。

## `status` 命令输出指标速览

当前的 `code-nav status` 会先展示 Master 进程自身的指标，然后按项目输出 Worker 概览表：

```
CodeNav Master Status
PID:        12345
Uptime:     1h 3m 12s
Workers:    2
ID          PID    PATH                   STATUS      INDEXED     UPTIME
proj-a      20001  /path/to/proj-a        "running"   120 files   52m 1s
proj-b      20002  /path/to/proj-b        "indexing"  42 files    12m 8s
```

- **Master**：输出 PID、运行时长以及当前管理的 Worker 数量。
- **Worker 表格**：针对每个项目列出 `ID`、`PID`、`PATH`、`STATUS`、已索引文件数以及运行时间，便于快速核对状态。
