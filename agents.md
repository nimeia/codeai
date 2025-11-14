下面为你生成一个**更精简、更专注于功能 / 技术栈 / 功能模块描述**的 `agents.md`，完全为 AI 代理提供项目背景信息，不包含实现细节或开发流程，适合作为智能代码助手的“项目上下文入口”。

---

# 📄 `agents.md`（精简版，专注功能 + 技术栈 + 模块结构）

```markdown
# code-nav / 项目总览（面向 AI Agents）

## 1. 项目简介

code-nav 是一个 **Rust 开发的本地代码智能导航与搜索工具**，用于在本机代码仓库中进行：

- 结构化代码索引（文件 / 目录 / 类 / 方法）
- 自然语言语义搜索（通过 embedding + 向量搜索）
- 代码快速定位（goto）
- 实时增量索引（监听文件变化）
- 多语言 AST 解析（基于 tree-sitter）
- 终端下浏览项目结构（tree / list）

系统由 **服务端守护进程（code-navd）** 与 **命令行客户端（code-nav）** 两部分组成。

目标：  
提供 VSCode + Sourcegraph 级别的代码搜索能力，但完全离线、本地、高性能、轻量。

---

## 2. 技术栈（Technology Stack）

### 核心语言
- **Rust**（高性能、零 GC、适合长期常驻服务）

### AST 解析
- **tree-sitter**（多语言解析）
  - 支持 Java / Rust / TS / JS / Python 等

### 嵌入模型（Embedding）
- **本地模式：candle / ggml / onnxruntime**
- **远程模式：OpenAI / DeepSeek / 内网模型 API**

### 向量搜索（ANN）
- **hnsw_rs（Rust 原生 HNSW 实现）**
- 或可选：lancedb、sqlite-vector、qdrant embedded

### 存储层
- **SQLite**（存储 symbol / 文件结构）
- 文件系统 + `.code-nav/` 工作目录

### 服务端通信协议
- **Unix Domain Socket**（推荐模式）
- 或 HTTP / JSON-RPC（可选）

### 运行模式
- 守护进程（daemon）
- 薄客户端 CLI

---

## 3. 系统主要功能（Features）

### 3.1 项目索引（Indexing）
- 扫描项目源码
- 使用 tree-sitter 解析语法树
- 抽取结构化符号：
  - 文件
  - 目录
  - 类 / 结构体
  - 方法 / 函数
  - 文档注释
- 全量索引 + 增量索引
- 文件变化监听（watcher）

### 3.2 语义搜索（Semantic Search）
- 自然语言 → 向量 embedding
- ANN 向量检索（HNSW）
- 返回最相关类/方法/文件
- 支持 top-k、范围过滤、语言过滤等

### 3.3 结构化搜索
- 列出类：`code-nav list classes`
- 列出方法：`code-nav list methods`
- 列出文件：`code-nav list files`
- 获取目录树：`code-nav tree`

### 3.4 代码跳转（Goto）
- 自然语言定位具体方法或类
- 返回：
  - 文件路径
  - 行号
  - 概要内容
- 可集成编辑器打开命令

### 3.5 服务端守护进程（code-navd）
- 常驻后台，保持加载索引与模型
- 接收 CLI 请求并执行：
  - semantic search
  - class/method list
  - goto 搜索
  - 索引刷新
- 管理索引 / 向量库 / 状态

---

## 4. 组件模块结构（Module Overview）

```

crates/
├── core/                      # 核心引擎
│   ├── indexer/               # AST 解析 & 扫描
│   ├── watcher/               # 文件变化监听
│   ├── metadata/              # SQLite 存储
│   ├── embedding/             # embedding 生成
│   ├── vectorstore/           # ANN 向量搜索
│   └── search/                # 搜索算法（语义+结构）
│
├── server/                    # code-navd 守护进程
│   ├── api/                   # RPC/Socket API
│   ├── daemon/                # 进程守护 & 服务管理
│   └── state/                 # 索引、模型、缓存
│
├── cli/                       # 客户端（code-nav）
│   ├── commands/              # search/goto/list/tree
│   ├── client/                # 与服务端通信
│   └── formatter/             # 输出格式化
│
└── protocol/                  # RPC 请求/响应结构体

```

---

## 5. 服务端 API（能力清单）

服务端提供以下 RPC 能力：

| API | 描述 |
|------|---------|
| `/search` | 自然语言语义搜索 |
| `/goto` | 定位类/方法（语义搜索） |
| `/list/classes` | 列出所有类 |
| `/list/methods` | 列出所有方法 |
| `/list/files` | 列出所有文件 |
| `/tree` | 输出项目目录结构 |
| `/index/full` | 全量重建索引 |
| `/index/incremental` | 增量索引更新 |
| `/status` | 服务状态 |
| `/info` | 系统信息（索引版本、模型等） |

---

## 6. 工作目录结构（运行时）

服务端会在项目根目录创建：

```

.code-nav/
├── metadata.db      # SQLite，结构化索引
├── hnsw.index       # 向量库（ANN 索引）
└── config.json      # 项目配置

```

---

## 7. 非功能特性（NFR）

- **高性能**（embedding + HNSW 搜索 < 200ms）
- **轻量**（Rust 单文件可执行 + 小型模型）
- **离线可用**（本地 embedding）
- **可扩展**（未来加入调用链分析、LLM 解释等）
- **跨平台**（Linux / macOS / Windows）

---

## 8. 项目目标总结（让 AI 理解）

> code-nav 是一个“Rust 版本地 Sourcegraph + VSCode 智能搜索系统”，  
> 通过 AST + embedding + 向量检索，实现结构化 + 语义的深层代码搜索与快速导航。  
>  
> 系统使用服务端守护进程保持索引和模型常驻，CLI 调用服务端执行操作。

```

主要关键字（AI 解析用）:
Rust, code search, code navigation, AST, tree-sitter,
embedding, semantic search, HNSW, vector index,
daemon, CLI, SQLite, real-time indexing

```
```

请所有 AI Agents 在编写代码时遵守上述架构：

* core 负责所有逻辑
* server 只做守护 + API
* cli 只做交互 + 输出
* protocol 统一数据结构

```
```

