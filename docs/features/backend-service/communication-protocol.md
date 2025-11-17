# 设计方案：可插拔的客户端/服务端通信

为了同时支持本地 IPC (UDS) 和跨主机的 HTTP 通信，我们需要对通信层进行抽象化改造。

## 1. 核心设计思想

引入一个统一的 `RpcClient` trait（接口），将通信方式（UDS 或 HTTP）的具体实现细节与上层业务逻辑（如 `info`, `status` 等命令的处理）解耦。客户端将根据配置来决定是实例化一个 `UnixSocketClient` 还是 `HttpClient`。

## 2. 配置方案

### 服务端 (`code-navd`)

在服务端的配置文件中，`listen` 字段将支持配置一个或多个监听地址。

- **示例**:
  ```toml
  # 方案 A: 只监听本地 UDS (默认)
  listen = ["unix:///var/run/code-nav/master.sock"]

  # 方案 B: 同时监听 UDS 和 HTTP，用于本地和远程访问
  listen = [
      "unix:///var/run/code-nav/master.sock",
      "http://0.0.0.0:9090"
  ]
  ```

### 客户端 (`code-nav` CLI)

客户端将通过全局配置文件 (`~/.code-nav/client.toml`) 或 `--connect` 命令行参数来指定要连接的服务端地址。

- **默认连接地址**: `http://localhost:6688` (用于本地测试和开发)
- **示例**:
  ```bash
  # 连接到远程服务器
  code-nav --connect http://remote.server:9090 info

  # 连接到本地 UDS
  code-nav --connect unix:///var/run/code-nav/master.sock status

  # 使用默认 HTTP 地址
  code-nav info
  ```
- 如果未指定 `--connect`，客户端将使用默认的 `http://localhost:6688` 地址。

## 3. 客户端实现 (`cli` crate)

### 引入 `reqwest` 依赖

为了支持 HTTP 请求，我们将在 `crates/cli/Cargo.toml` 中添加 `reqwest` 依赖，它是一个强大且流行的 HTTP 客户端库。

### 重构 `client/mod.rs`

- 首先，定义一个 `RpcClient` trait，它只有一个核心方法 `send`。
  ```rust
  use code_nav_protocol::{Request, Response};
  use anyhow::Result;

  pub trait RpcClient {
      fn send(&self, request: &Request) -> Result<Response>;
  }
  ```
- 然后，提供两种具体的实现：
  ```rust
  // 1. UDS 实现 (用于本地)
  pub struct UnixSocketClient { /* ... */ }
  impl RpcClient for UnixSocketClient {
      fn send(&self, request: &Request) -> Result<Response> {
          // 内部逻辑：连接 socket、序列化请求、发送、接收响应、反序列化
      }
  }

  // 2. HTTP 实现 (用于远程)
  pub struct HttpClient { /* ... */ }
  impl RpcClient for HttpClient {
      fn send(&self, request: &Request) -> Result<Response> {
          // 内部逻辑：使用 reqwest 向指定 URL POST JSON 格式的请求，
          // 然后将收到的 JSON 响应反序列化。
      }
  }
  ```
- 最后，提供一个工厂函数，根据地址格式（`http://` 或 `unix://`）创建对应的 `RpcClient` 实例。

### 修改命令处理逻辑

`main.rs` 和 `project.rs` 中的 `handle_*` 函数将不再关心如何发送请求。程序启动时会根据配置初始化一个 `Box<dyn RpcClient>`，并将其传递给需要与服务端通信的函数。

## 4. 服务端实现 (`server` crate)

### 引入 `axum` 和 `tokio` 依赖

为了构建高性能的异步服务，服务端将基于 `tokio` 运行时，并使用 `axum` 作为 Web 框架来处理 HTTP 请求。

### 统一请求处理器

创建一个核心的 `handle_rpc_request(request: Request) -> Response` 函数。所有业务逻辑都集中于此。

### 启动多种监听器

服务端启动时，会根据配置启动一个或多个监听任务。

- **HTTP 监听器**: 启动一个 `axum` 服务器，设置一个 `/rpc` 路由。所有发往此路由的 `POST` 请求都会被 `handle_rpc_request` 函数处理。
- **UDS 监听器**: 启动一个循环来监听 UDS 连接。每当有新的客户端连接时，它会读取数据，调用相同的 `handle_rpc_request` 函数，然后将结果写回。
