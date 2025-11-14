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

后续可在各 crate 中逐步替换占位实现，接入 tree-sitter、embedding 后端、HNSW 向量库等真实逻辑。
