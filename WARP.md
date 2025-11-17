# WARP.md

This file provides guidance to WARP (warp.dev) when working with code in this repository.

## Project Overview

**code-nav** is a Rust-based local code intelligence navigation and search tool that provides:
- Structured code indexing (files/directories/classes/methods)
- Natural language semantic search (via embeddings + vector search)
- Code location/navigation (goto)
- Real-time incremental indexing with file watching
- Multi-language AST parsing (using tree-sitter)

The system consists of a **server daemon (code-navd)** and a **CLI client (code-nav)** communicating via Unix Domain Sockets or named pipes.

## Development Commands

### Building
```bash
# Format code
cargo fmt

# Build all crates
cargo build

# Build with release optimizations
cargo build --release

# Build specific crate
cargo build -p code-nav-cli
cargo build -p code-nav-server
cargo build -p code-nav-core
cargo build -p code-nav-protocol
```

### Running
```bash
# Run the CLI client
cargo run -p code-nav-cli -- search "query text" 5
cargo run -p code-nav-cli -- list classes

# Run the server daemon
cargo run -p code-nav-server -- start
cargo run -p code-nav-server -- start --foreground
cargo run -p code-nav-server -- stop
cargo run -p code-nav-server -- restart
```

### Testing
```bash
# Run all tests
cargo test

# Run tests for specific crate
cargo test -p code-nav-core

# Run with output
cargo test -- --nocapture
```

### Linting
```bash
# Check code without building
cargo check

# Run clippy linter
cargo clippy

# Run clippy with all targets
cargo clippy --all-targets
```

## Architecture

### Workspace Structure

This is a Cargo workspace with 4 crates:

```
crates/
├── protocol/       # RPC request/response structures (shared types)
├── core/           # Core indexing, search, and embedding logic
├── server/         # Daemon process (code-navd) with master/worker management
└── cli/            # Command-line client (code-nav)
```

### Dependency Flow
```
cli → protocol
server → core → protocol
```

### Key Architectural Principles

**Separation of Concerns:**
- **`protocol`**: Pure data structures for communication. No business logic.
- **`core`**: All indexing, parsing, embedding, and search algorithms. No I/O or networking.
- **`server`**: Daemon lifecycle, API endpoints, state management. Orchestrates `core` modules.
- **`cli`**: User interface, command parsing, formatting output. Thin client that delegates to server.

**Core Modules** (`crates/core/src/`):
- `indexer/`: AST parsing and code scanning (placeholder for tree-sitter integration)
- `watcher/`: File system change monitoring for incremental updates
- `metadata/`: SQLite-based structured index storage
- `embedding/`: Text → vector conversion (local or remote model)
- `vectorstore/`: HNSW-based approximate nearest neighbor search
- `search/`: High-level semantic and structured search combining above modules

**Server Architecture:**
- `master/`: Daemon lifecycle (start/stop/restart), PID management, lock files
- `daemon/`: Long-running service process coordination
- `api/`: RPC endpoint handlers for protocol requests
- `state/`: In-memory cache of indexes, models, and active projects

### Protocol Design

Uses tagged enum for type-safe RPC:
```rust
Request::Search(SearchRequest) → Response::Search(SearchResponse)
Request::List(ListRequest) → Response::List(ListResponse)
Request::Goto(GotoRequest) → Response::Goto(GotoResponse)
```

All requests/responses are JSON serialized.

### Runtime Directory

Server creates `.code-nav/` in project root:
```
.code-nav/
├── metadata.db      # SQLite index
├── hnsw.index       # Vector store
├── config.json      # Project config
└── master.pid       # Daemon PID
```

## Implementation Notes

### Current State
This is an **early-stage skeleton**. Most modules contain placeholder implementations:
- `embedding::embed()` returns dummy vectors
- `indexer::run()` logs but doesn't parse
- No actual tree-sitter integration yet
- No real HNSW implementation yet
- Server daemon runs but doesn't process requests

### When Adding Features

**For tree-sitter integration:**
- Add parsers to `core/indexer/`
- Create language-specific modules (e.g., `indexer/rust.rs`, `indexer/java.rs`)
- Extract symbols (classes, methods, comments) into structured format
- Store in `metadata/` SQLite schema

**For embedding models:**
- Add model loading logic to `core/embedding/`
- Support both local (candle/onnxruntime) and remote (API) modes
- Consider lazy loading and caching
- Models should be loaded once by server, not per-request

**For vector search:**
- Implement HNSW in `core/vectorstore/` or integrate `hnsw_rs` crate
- Design index serialization format for `.code-nav/hnsw.index`
- Support incremental updates (add/remove vectors)

**For server API:**
- Implement handlers in `server/api/` that call `core/search`
- Ensure proper error handling using `protocol::ErrorCode`
- Keep handlers thin - delegate to `core` modules

**For CLI commands:**
- Add new `Commands` variants in `cli/src/main.rs`
- Convert to `protocol::Request` enum
- Format responses from server for terminal display in `cli/formatter/`

### Windows Considerations

This codebase targets Windows (current working directory is Windows):
- Use named pipes instead of Unix domain sockets for IPC
- Handle path separators properly (prefer `PathBuf` operations)
- Test daemon process management on Windows (PID handling differs from Unix)
- The `windows-sys` dependency is already included for platform-specific APIs

### Communication Pattern

1. CLI parses user command → creates `Request` enum
2. CLI sends JSON-serialized request to server socket/pipe
3. Server deserializes → routes to appropriate handler
4. Handler uses `core` modules → builds `Response`
5. Server serializes response → sends back to CLI
6. CLI formats and displays to user

Currently step 2-5 are not implemented (CLI just prints JSON request).

## Coding Style

- Use workspace dependencies defined in root `Cargo.toml`
- All crates use edition 2021
- Prefer `anyhow::Result` for CLI/server, `thiserror` for library errors
- Use `tracing` for logging, not `println!` in library code
- Chinese comments are used in some parts of the codebase (especially server daemon messages)
