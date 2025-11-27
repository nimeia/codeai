use std::{
    collections::{hash_map::DefaultHasher, BTreeMap},
    ffi::OsStr,
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{self, Write},
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use axum::{extract::State, routing::post, Json, Router};
use chrono::Utc;
use clap::ValueEnum;
use code_nav_core::indexer::{self, IndexJob, IndexProgress, IndexReport, IndexRunMode};
use code_nav_protocol::{
    ErrorBody, ErrorCode, GotoRequest, GotoResponse, IndexMode, IndexRequest, IndexResponse,
    IndexerState, InfoRequest, InfoResponse, ListItem, ListKind, ListRequest, ListResponse,
    LogsRequest, MasterInfo, MasterStatus, ProjectAddRequest, ProjectAddResponse, ProjectInfo,
    ProjectListRequest, ProjectListResponse, ProjectRef, ProjectRemoveRequest,
    ProjectRemoveResponse, ProjectRestartRequest, ProjectRestartResponse, ProjectStatusRequest,
    ProjectStatusResponse, Request, Response, SearchRequest, SearchResponse, StatusRequest,
    StatusResponse, WorkerInfo, WorkerState, WorkerStatus, WorkerSummary,
};
use fslock::LockFile;
use is_terminal::IsTerminal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, info, warn, Level};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::time::UtcTime;

mod logs;

#[derive(Debug)]
pub struct StartCommand {
    pub config_path: Option<PathBuf>,
    pub runtime_dir: Option<PathBuf>,
    pub socket: Option<String>,
    pub log_level: Option<LogLevel>,
    pub log_dir: Option<PathBuf>,
    pub worker_log_spool: Option<PathBuf>,
    pub foreground: bool,
    pub grace: Option<u64>,
    pub autostart: AutostartMode,
    pub wait_ready: bool,
    pub ready_timeout: Duration,
}

#[derive(Debug)]
pub struct StopCommand {
    pub config_path: Option<PathBuf>,
    pub runtime_dir: Option<PathBuf>,
    pub socket: Option<String>,
    pub grace_secs: u64,
    pub timeout_secs: u64,
    pub force: bool,
    pub assume_yes: bool,
}

#[derive(Debug)]
pub struct RestartCommand {
    pub start: StartCommand,
    pub stop: StopCommand,
}

#[derive(Clone)]
struct MasterState {
    registry: Arc<Mutex<ProjectRegistry>>,
    runtime: Arc<Mutex<BTreeMap<String, ProjectRuntimeEntry>>>,
    workers: Arc<Mutex<BTreeMap<String, WorkerRuntimeEntry>>>,
    runtime_dir: PathBuf,
    worker_log_spool: PathBuf,
}

pub async fn run_start(args: StartCommand) -> Result<()> {
    let config = MasterConfig::load(&args)?;
    let _lock = MasterLock::acquire(&config)?;
    let logging = LoggingGuards::init(&config)?;

    let registry = Arc::new(Mutex::new(ProjectRegistry::load(&config)?));
    let runtime = Arc::new(Mutex::new(BTreeMap::<String, ProjectRuntimeEntry>::new()));
    let workers = Arc::new(Mutex::new(BTreeMap::<String, WorkerRuntimeEntry>::new()));

    if args.wait_ready {
        debug!(
            wait_timeout_secs = args.ready_timeout.as_secs(),
            "wait-ready enabled (placeholder until background mode is implemented)"
        );
    } else {
        warn!("--no-wait 指定，但当前 master 仍以前台模式运行");
    }

    if !config.foreground {
        warn!("后台模式尚未实现，当前进程将以前台模式运行");
    }

    {
        let registry_guard = registry
            .lock()
            .map_err(|_| anyhow!("failed to acquire registry lock"))?;
        apply_autostart(&config, &registry_guard)?;
        print_start_feedback(&config, &registry_guard);
    }

    run_main_loop(&config, logging, registry, runtime, workers).await
}

pub fn run_stop(args: StopCommand) -> Result<()> {
    let config = StopConfig::load(&args)?;

    if !config.runtime_dir.exists() {
        println!(
            "code-navd 未在运行（未找到运行目录 {:?}）",
            config.runtime_dir
        );
        return Ok(());
    }

    let pid_info = match read_pid_file(&config.pid_path) {
        Ok(info) => info,
        Err(err)
            if err.downcast_ref::<io::Error>().map(|e| e.kind())
                == Some(io::ErrorKind::NotFound) =>
        {
            println!("code-navd 未在运行（未找到 PID 文件）");
            cleanup_runtime_artifacts(&config)?;
            return Ok(());
        }
        Err(err) => {
            return Err(err).context("无法读取 master.pid");
        }
    };

    let pid = match pid_info.pid {
        Some(pid) => pid,
        None => {
            println!("PID 文件缺少进程信息，视为未运行");
            cleanup_runtime_artifacts(&config)?;
            return Ok(());
        }
    };

    if !config.assume_yes && !confirm_stop()? {
        println!("已取消停止操作");
        return Ok(());
    }

    if !is_process_alive(pid) {
        println!("code-navd 不在运行（PID {pid} 不存在），清理残留文件");
        cleanup_runtime_artifacts(&config)?;
        return Ok(());
    }

    println!("发送停止请求（PID {pid}）...");
    send_signal(pid, false)?;

    let start = Instant::now();
    let mut forced = false;

    loop {
        if !is_process_alive(pid) {
            println!("停止成功，PID {pid} 已退出");
            cleanup_runtime_artifacts(&config)?;
            return Ok(());
        }

        if config.force && !forced && start.elapsed() >= config.grace {
            println!("宽限期已结束，发送强制终止信号...");
            send_signal(pid, true)?;
            forced = true;
        }

        if start.elapsed() >= config.timeout {
            if config.force && forced {
                return Err(anyhow!(
                    "等待守护进程退出超时（即使已强制终止），请检查 PID {pid}"
                ));
            } else {
                return Err(anyhow!("等待守护进程退出超时，可重试或使用 --force 参数"));
            }
        }

        thread::sleep(Duration::from_millis(500));
    }
}

pub async fn run_restart(cmd: RestartCommand) -> Result<()> {
    println!("准备停止 code-navd...");
    run_stop(cmd.stop)?;
    println!("守护进程已停止，准备重新启动...");
    run_start(cmd.start).await
}

async fn run_main_loop(
    config: &MasterConfig,
    _logging: LoggingGuards,
    registry: Arc<Mutex<ProjectRegistry>>,
    runtime: Arc<Mutex<BTreeMap<String, ProjectRuntimeEntry>>>,
    workers: Arc<Mutex<BTreeMap<String, WorkerRuntimeEntry>>>,
) -> Result<()> {
    let state = MasterState {
        registry,
        runtime,
        workers,
        runtime_dir: config.runtime_dir.clone(),
        worker_log_spool: config.worker_log_spool.clone(),
    };
    let shutdown = Arc::new(AtomicBool::new(false));
    let signal_flag = shutdown.clone();
    ctrlc::set_handler(move || {
        signal_flag.store(true, Ordering::SeqCst);
    })
    .context("failed to install ctrl-c handler")?;

    let cors = CorsLayer::new().allow_origin(Any);
    let app = Router::new()
        .route("/", post(rpc_handler))
        .route("/rpc", post(rpc_handler))
        .route("/info", post(info_handler))
        .route("/status", post(status_handler))
        .route("/search", post(search_handler))
        .route("/goto", post(goto_handler))
        .route("/list", post(list_handler))
        .route("/list/classes", post(list_classes_handler))
        .route("/list/methods", post(list_methods_handler))
        .route("/list/files", post(list_files_handler))
        .route("/list/tree", post(list_tree_handler))
        .route("/index", post(index_handler))
        .route("/index/full", post(index_full_handler))
        .route("/index/incremental", post(index_incremental_handler))
        .route("/logs", post(logs_handler))
        .route("/project/add", post(project_add_handler))
        .route("/project/remove", post(project_remove_handler))
        .route("/project/list", post(project_list_handler))
        .route("/project/status", post(project_status_handler))
        .route("/project/restart", post(project_restart_handler))
        .with_state(state)
        .layer(cors);

    let mut listener_tasks = Vec::new();

    for socket_addr_kind in &config.listen {
        let app = app.clone();
        match socket_addr_kind {
            SocketAddrKind::Tcp(addr) => {
                info!("HTTP server listening on {}", addr);
                let listener = TcpListener::bind(addr).await?;
                listener_tasks.push(tokio::spawn(async move {
                    axum::serve(listener, app).await.unwrap();
                }));
            }
            SocketAddrKind::Unix(path) => {
                info!("UDS server listening on {}", path.display());
                let path_clone = path.clone();
                listener_tasks.push(tokio::spawn(async move {
                    run_uds_listener(path_clone, app).await.unwrap();
                }));
            }
            SocketAddrKind::NamedPipe(name) => {
                info!("Named pipe server listening on {}", name);
                // TODO: Implement named pipe listener
                listener_tasks.push(tokio::spawn(async move {
                    // Placeholder for named pipe listener
                    tokio::signal::ctrl_c().await.unwrap();
                }));
            }
        }
    }

    // Wait for all listener tasks to complete or for a shutdown signal
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("shutdown requested, exiting master loop");
        }
        _ = futures::future::join_all(listener_tasks) => {
            info!("all listeners stopped, exiting master loop");
        }
    }

    Ok(())
}

async fn run_uds_listener(path: PathBuf, _app: Router) -> Result<()> {
    // Remove the socket file if it already exists
    if path.exists() {
        fs::remove_file(&path).context("failed to remove existing UDS file")?;
    }

    let listener = tokio::net::UnixListener::bind(&path)
        .with_context(|| format!("failed to bind to UDS path {}", path.display()))?;

    info!("UDS listener started on {}", path.display());

    loop {
        match listener.accept().await {
            Ok((_stream, _addr)) => {
                debug!("Accepted UDS connection");
                // For now, just accept and close.
                // TODO: Implement actual RPC handling for UDS
            }
            Err(e) => {
                warn!("UDS accept error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

async fn rpc_handler(
    State(state): State<MasterState>,
    Json(request): Json<Request>,
) -> Json<Response> {
    tracing::debug!(?request, "received request");
    let response = handle_rpc_request(&state, request);
    Json(response)
}

async fn info_handler(
    State(state): State<MasterState>,
    Json(req): Json<InfoRequest>,
) -> Json<Response> {
    Json(handle_rpc_request(&state, Request::Info(req)))
}

async fn status_handler(
    State(state): State<MasterState>,
    Json(req): Json<StatusRequest>,
) -> Json<Response> {
    Json(handle_rpc_request(&state, Request::Status(req)))
}

async fn search_handler(
    State(state): State<MasterState>,
    Json(req): Json<SearchRequest>,
) -> Json<Response> {
    Json(handle_rpc_request(&state, Request::Search(req)))
}

async fn goto_handler(
    State(state): State<MasterState>,
    Json(req): Json<GotoRequest>,
) -> Json<Response> {
    Json(handle_rpc_request(&state, Request::Goto(req)))
}

async fn list_handler(
    State(state): State<MasterState>,
    Json(req): Json<ListRequest>,
) -> Json<Response> {
    Json(handle_rpc_request(&state, Request::List(req)))
}

async fn list_classes_handler(
    State(state): State<MasterState>,
    Json(req): Json<PartialListRequest>,
) -> Json<Response> {
    let req = ListRequest {
        kind: ListKind::Classes,
        filter: req.filter,
        limit: req.limit,
    };
    Json(handle_rpc_request(&state, Request::List(req)))
}

async fn list_methods_handler(
    State(state): State<MasterState>,
    Json(req): Json<PartialListRequest>,
) -> Json<Response> {
    let req = ListRequest {
        kind: ListKind::Methods,
        filter: req.filter,
        limit: req.limit,
    };
    Json(handle_rpc_request(&state, Request::List(req)))
}

async fn list_files_handler(
    State(state): State<MasterState>,
    Json(req): Json<PartialListRequest>,
) -> Json<Response> {
    let req = ListRequest {
        kind: ListKind::Files,
        filter: req.filter,
        limit: req.limit,
    };
    Json(handle_rpc_request(&state, Request::List(req)))
}

async fn list_tree_handler(
    State(state): State<MasterState>,
    Json(req): Json<PartialListRequest>,
) -> Json<Response> {
    let req = ListRequest {
        kind: ListKind::Tree,
        filter: req.filter,
        limit: req.limit,
    };
    Json(handle_rpc_request(&state, Request::List(req)))
}

async fn index_handler(
    State(state): State<MasterState>,
    Json(req): Json<IndexRequest>,
) -> Json<Response> {
    Json(handle_rpc_request(&state, Request::Index(req)))
}

async fn index_full_handler(State(state): State<MasterState>) -> Json<Response> {
    Json(handle_rpc_request(
        &state,
        Request::Index(IndexRequest {
            mode: IndexMode::Full,
        }),
    ))
}

async fn index_incremental_handler(State(state): State<MasterState>) -> Json<Response> {
    Json(handle_rpc_request(
        &state,
        Request::Index(IndexRequest {
            mode: IndexMode::Incremental,
        }),
    ))
}

async fn logs_handler(
    State(state): State<MasterState>,
    Json(req): Json<LogsRequest>,
) -> Json<Response> {
    Json(handle_rpc_request(&state, Request::Logs(req)))
}

async fn project_add_handler(
    State(state): State<MasterState>,
    Json(req): Json<ProjectAddRequest>,
) -> Json<Response> {
    Json(handle_rpc_request(&state, Request::ProjectAdd(req)))
}

async fn project_remove_handler(
    State(state): State<MasterState>,
    Json(req): Json<ProjectRemoveRequest>,
) -> Json<Response> {
    Json(handle_rpc_request(&state, Request::ProjectRemove(req)))
}

async fn project_list_handler(
    State(state): State<MasterState>,
    Json(req): Json<ProjectListRequest>,
) -> Json<Response> {
    Json(handle_rpc_request(&state, Request::ProjectList(req)))
}

async fn project_status_handler(
    State(state): State<MasterState>,
    Json(req): Json<ProjectStatusRequest>,
) -> Json<Response> {
    Json(handle_rpc_request(&state, Request::ProjectStatus(req)))
}

async fn project_restart_handler(
    State(state): State<MasterState>,
    Json(req): Json<ProjectRestartRequest>,
) -> Json<Response> {
    Json(handle_rpc_request(&state, Request::ProjectRestart(req)))
}

fn handle_rpc_request(state: &MasterState, request: Request) -> Response {
    match request {
        Request::Info(req) => match req
            .target
            .as_ref()
            .and_then(|project| state.resolve_project(project))
        {
            Some(project) => match state.worker_status(&project.project_id) {
                Some(worker) => Response::Info(InfoResponse::Worker(WorkerInfo {
                    protocol_version: "1.0".to_string(),
                    server_version: "0.1.0".to_string(),
                    project_id: worker.summary.project_id.clone(),
                    project_root: worker.summary.project_root.clone(),
                    pid: worker.summary.pid,
                    uptime_secs: worker.summary.uptime_secs,
                    config_summary: BTreeMap::new(),
                })),
                None => Response::Error(ErrorBody {
                    code: ErrorCode::NotIndexed,
                    message: format!(
                        "project {} is registered but no worker is active",
                        project.project_id
                    ),
                }),
            },
            None => match state.master_status() {
                Ok(status) => Response::Info(InfoResponse::Master(MasterInfo {
                    protocol_version: "1.0".to_string(),
                    server_version: "0.1.0".to_string(),
                    pid: status.pid,
                    uptime_secs: status.uptime_secs,
                    projects_managed: status.workers.len(),
                })),
                Err(err) => Response::Error(ErrorBody {
                    code: ErrorCode::InternalError,
                    message: err.to_string(),
                }),
            },
        },
        Request::Search(_req) => Response::Search(SearchResponse { hits: vec![] }),
        Request::Goto(_req) => Response::Goto(GotoResponse {
            file: None,
            line: None,
        }),
        Request::List(req) => match req.kind {
            ListKind::Classes => Response::List(ListResponse {
                items: vec![ListItem {
                    name: "ExampleClass".into(),
                    location: Some("src/example.rs:1".into()),
                }],
            }),
            ListKind::Methods => Response::List(ListResponse {
                items: vec![ListItem {
                    name: "example_method".into(),
                    location: Some("src/example.rs:10".into()),
                }],
            }),
            ListKind::Files => Response::List(ListResponse {
                items: vec![ListItem {
                    name: "src/example.rs".into(),
                    location: None,
                }],
            }),
            ListKind::Tree => Response::List(ListResponse {
                items: vec![ListItem {
                    name: "/".into(),
                    location: None,
                }],
            }),
        },
        Request::Index(_req) => Response::Index(IndexResponse { started: true }),
        Request::Status(req) => match req
            .target
            .as_ref()
            .and_then(|project| state.resolve_project(project))
        {
            Some(project) => match state.worker_status(&project.project_id) {
                Some(worker) => Response::Status(StatusResponse::Worker(worker)),
                None => Response::Error(ErrorBody {
                    code: ErrorCode::NotIndexed,
                    message: format!(
                        "project {} is registered but worker has not started",
                        project.project_id
                    ),
                }),
            },
            None => match state.master_status() {
                Ok(status) => Response::Status(StatusResponse::Master(status)),
                Err(err) => Response::Error(ErrorBody {
                    code: ErrorCode::InternalError,
                    message: err.to_string(),
                }),
            },
        },
        Request::Logs(req) => match logs::global() {
            Some(service) => service.handle_request(req),
            None => Response::Error(ErrorBody {
                code: ErrorCode::InternalError,
                message: "日志服务尚未初始化".to_string(),
            }),
        },
        Request::ProjectAdd(req) => match state.register_project(req) {
            Ok(response) => Response::ProjectAdd(response),
            Err(err) => Response::Error(ErrorBody {
                code: ErrorCode::InvalidRequest,
                message: format!("failed to register project: {err}"),
            }),
        },
        Request::ProjectRemove(req) => Response::ProjectRemove(ProjectRemoveResponse {
            project_id: match &req.project {
                ProjectRef::Path(path) => format!("proj-{path}"),
                ProjectRef::Id(id) => id.clone(),
            },
            project_root: match &req.project {
                ProjectRef::Path(path) => PathBuf::from(path),
                ProjectRef::Id(id) => PathBuf::from(id),
            },
        }),
        Request::ProjectList(_req) => match state.list_projects() {
            Ok(projects) => Response::ProjectList(ProjectListResponse { projects }),
            Err(err) => Response::Error(ErrorBody {
                code: ErrorCode::InternalError,
                message: err.to_string(),
            }),
        },
        Request::ProjectStatus(req) => match state.project_status(&req.project) {
            Ok(info) => Response::ProjectStatus(ProjectStatusResponse { info }),
            Err(err) => Response::Error(ErrorBody {
                code: ErrorCode::InvalidRequest,
                message: err.to_string(),
            }),
        },
        Request::ProjectRestart(req) => match state.restart_projects(req.projects) {
            Ok(projects) => Response::ProjectRestart(ProjectRestartResponse { projects }),
            Err(err) => Response::Error(ErrorBody {
                code: ErrorCode::InvalidRequest,
                message: err.to_string(),
            }),
        },
    }
}

impl MasterState {
    fn register_project(&self, req: ProjectAddRequest) -> Result<ProjectAddResponse> {
        let (entry, duplicate) = self.register_in_registry(&req)?;
        let info = self.ensure_runtime_entry(&entry, Some(&req))?;
        let state_label = if duplicate { "duplicate" } else { "starting" }.to_string();
        self.update_project_state(&entry.project_id, &state_label, None)?;

        if !duplicate {
            self.spawn_worker(entry.clone(), req)?;
        }

        Ok(ProjectAddResponse {
            project_id: entry.project_id,
            project_root: info.project_root,
            state: state_label,
        })
    }

    fn register_in_registry(&self, req: &ProjectAddRequest) -> Result<(RegistryEntry, bool)> {
        let canonical = fs::canonicalize(&req.project_root).with_context(|| {
            format!("project path {} does not exist", req.project_root.display())
        })?;
        let metadata = fs::metadata(&canonical)
            .with_context(|| format!("failed to read metadata for {}", canonical.display()))?;
        if !metadata.is_dir() {
            bail!("{} is not a directory", canonical.display());
        }

        let mut registry = self
            .registry
            .lock()
            .map_err(|_| anyhow!("failed to acquire registry lock"))?;

        if let Some(existing) = registry.contains_root_mut(&canonical) {
            let mut updated = false;
            if existing.autostart != req.autostart {
                existing.autostart = req.autostart;
                updated = true;
            }
            if existing.watch != req.watch {
                existing.watch = req.watch;
                updated = true;
            }
            if existing.index_mode != req.index_mode {
                existing.index_mode = req.index_mode.clone();
                updated = true;
            }
            if existing.model != req.model {
                existing.model = req.model.clone();
                updated = true;
            }

            if updated {
                registry.persist()?;
                info!(
                    project_root = %canonical.display(),
                    project_id = %existing.project_id,
                    "project already registered; options updated",
                );
            } else {
                info!(
                    project_root = %canonical.display(),
                    project_id = %existing.project_id,
                    "project already registered",
                );
            }

            return Ok((existing.clone(), true));
        }
        if let Some(entry) = registry.contains_root(&canonical) {
            info!(
                project_root = %canonical.display(),
                project_id = %entry.project_id,
                "project already registered",
            );
            return Ok((entry.clone(), true));
        }

        let project_id = if let Some(custom) = req.id.clone() {
            if let Some(existing) = registry.contains_id(&custom) {
                bail!(
                    "project id {custom} already registered for {}",
                    existing.project_root.display()
                );
            }
            custom
        } else {
            generate_project_id(&canonical)
        };

        let entry = RegistryEntry {
            project_id: project_id.clone(),
            project_root: canonical.clone(),
            autostart: req.autostart,
            watch: req.watch,
            index_mode: req.index_mode.clone(),
            model: req.model.clone(),
            last_running: false,
            last_state: Some("pending".to_string()),
        };

        registry.add(entry.clone())?;

        info!(
            project_id = %project_id,
            project_root = %canonical.display(),
            "project registered and pending worker startup",
        );

        Ok((entry, false))
    }

    fn ensure_runtime_entry(
        &self,
        entry: &RegistryEntry,
        source: Option<&ProjectAddRequest>,
    ) -> Result<ProjectInfo> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| anyhow!("failed to acquire runtime state lock"))?;

        if let Some(existing) = runtime.get(&entry.project_id) {
            return Ok(existing.info.clone());
        }

        let info = ProjectInfo {
            project_id: entry.project_id.clone(),
            project_root: entry.project_root.clone(),
            autostart: source.map(|s| s.autostart).unwrap_or(entry.autostart),
            watch: source.map(|s| s.watch).unwrap_or(entry.watch),
            index_mode: source
                .map(|s| s.index_mode.clone())
                .unwrap_or(entry.index_mode.clone()),
            model: source
                .and_then(|s| s.model.clone())
                .or_else(|| entry.model.clone()),
            state: entry
                .last_state
                .clone()
                .unwrap_or_else(|| "pending".to_string()),
        };

        runtime.insert(
            entry.project_id.clone(),
            ProjectRuntimeEntry {
                info: info.clone(),
                last_error: None,
            },
        );

        Ok(info)
    }

    fn update_project_state(
        &self,
        project_id: &str,
        state: &str,
        last_error: Option<String>,
    ) -> Result<()> {
        let registry_entry = {
            let registry = self
                .registry
                .lock()
                .map_err(|_| anyhow!("failed to acquire registry lock"))?;
            registry.by_id(project_id).cloned()
        };

        {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| anyhow!("failed to acquire runtime state lock"))?;
            let project_root = registry_entry
                .as_ref()
                .map(|entry| entry.project_root.clone())
                .unwrap_or_default();
            let entry =
                runtime
                    .entry(project_id.to_string())
                    .or_insert_with(|| ProjectRuntimeEntry {
                        info: ProjectInfo {
                            project_id: project_id.to_string(),
                            project_root,
                            autostart: false,
                            watch: false,
                            index_mode: IndexMode::Auto,
                            model: None,
                            state: state.to_string(),
                        },
                        last_error: None,
                    });
            entry.info.state = state.to_string();
            entry.last_error = last_error.clone();
        }

        if let Some(_entry) = registry_entry {
            let mut registry = self
                .registry
                .lock()
                .map_err(|_| anyhow!("failed to acquire registry lock"))?;
            if let Some(reg) = registry.by_id_mut(project_id) {
                reg.last_state = Some(state.to_string());
            }
            registry.persist()?;
        }

        Ok(())
    }

    fn spawn_worker(&self, entry: RegistryEntry, req: ProjectAddRequest) -> Result<()> {
        {
            let mut workers = self
                .workers
                .lock()
                .map_err(|_| anyhow!("failed to acquire worker state lock"))?;
            if let Some(existing) = workers.get(&entry.project_id) {
                info!(
                    project_id = %entry.project_id,
                    state = ?existing.summary.status,
                    "worker already active, skip spawn",
                );
                return Ok(());
            }

            let summary = WorkerSummary {
                project_id: entry.project_id.clone(),
                project_root: entry.project_root.clone(),
                pid: std::process::id(),
                status: WorkerState::Starting,
                indexed_files_count: 0,
                uptime_secs: 0,
            };

            workers.insert(
                entry.project_id.clone(),
                WorkerRuntimeEntry {
                    summary,
                    started_at: Instant::now(),
                    last_error: None,
                    indexer_state: IndexerState {
                        state: WorkerState::Starting,
                        progress_percent: Some(0.0),
                        current_file: None,
                    },
                },
            );
        }

        self.update_project_state(&entry.project_id, "starting", None)?;
        self.persist_worker_state(&entry.project_id)?;

        let state = self.clone();
        let entry_for_bootstrap = entry.clone();
        tokio::spawn(async move {
            state.run_worker_bootstrap(entry_for_bootstrap).await;
        });

        debug!(
            project_id = %req.id.clone().unwrap_or_else(|| entry.project_id.clone()),
            watch = req.watch,
            autostart = req.autostart,
            index_mode = ?req.index_mode,
            "worker spawn scheduled",
        );

        Ok(())
    }

    async fn run_worker_bootstrap(&self, entry: RegistryEntry) {
        info!(
            project_id = %entry.project_id,
            project_root = %entry.project_root.display(),
            "starting worker"
        );

        if let Err(err) = self.prepare_worker_runtime(&entry) {
            warn!(
                project_id = %entry.project_id,
                error = %err,
                "failed to prepare worker runtime"
            );
            let _ = self.set_worker_state(
                &entry.project_id,
                WorkerState::Failed,
                Some(err.to_string()),
            );
            return;
        }

        if let Err(err) = self.set_worker_state(&entry.project_id, WorkerState::Indexing, None) {
            warn!(
                project_id = %entry.project_id,
                error = %err,
                "failed to mark worker as indexing",
            );
            return;
        }

        let project_id = entry.project_id.clone();
        let project_root = entry.project_root.clone();
        let state_for_progress = self.clone();
        let index_result = tokio::task::spawn_blocking(move || {
            let job = IndexJob::new(project_root).with_mode(IndexRunMode::Full);
            indexer::run_with_progress(job, |progress| {
                if let Err(err) = state_for_progress.update_index_progress(&project_id, progress) {
                    warn!(project_id = %project_id, error = %err, "failed to update index progress");
                }
            })
        })
        .await;

        match index_result {
            Ok(Ok(report)) => {
                if let Err(err) = self.apply_index_report(&entry.project_id, &report) {
                    warn!(
                        project_id = %entry.project_id,
                        error = %err,
                        "failed to persist index report",
                    );
                }
                if let Err(err) =
                    self.set_worker_state(&entry.project_id, WorkerState::Running, None)
                {
                    warn!(
                        project_id = %entry.project_id,
                        error = %err,
                        "failed to mark worker as running",
                    );
                    return;
                }
                if let Ok(mut registry) = self.registry.lock() {
                    if let Err(err) = registry.mark_running(&entry.project_id, true) {
                        warn!(
                            project_id = %entry.project_id,
                            error = %err,
                            "failed to persist running state"
                        );
                    }
                }

                info!(
                    project_id = %entry.project_id,
                    files_total = report.files_total,
                    symbols = report.symbols_indexed,
                    "worker is now running",
                );
            }
            Ok(Err(err)) => {
                warn!(
                    project_id = %entry.project_id,
                    error = %err,
                    "indexer failed",
                );
                let _ = self.set_worker_state(
                    &entry.project_id,
                    WorkerState::Failed,
                    Some(err.to_string()),
                );
            }
            Err(join_err) => {
                warn!(
                    project_id = %entry.project_id,
                    error = %join_err,
                    "indexer task join failed",
                );
                let _ = self.set_worker_state(
                    &entry.project_id,
                    WorkerState::Failed,
                    Some(join_err.to_string()),
                );
            }
        }
    }

    fn prepare_worker_runtime(&self, entry: &RegistryEntry) -> Result<()> {
        let project_dir = self.runtime_dir.join("projects").join(&entry.project_id);
        fs::create_dir_all(&project_dir)
            .with_context(|| format!("failed to create runtime dir {}", project_dir.display()))?;

        let worker_log = self
            .worker_log_spool
            .join(format!("{}.log", entry.project_id));
        if !worker_log.exists() {
            File::create(&worker_log).with_context(|| {
                format!(
                    "failed to initialize worker log spool {}",
                    worker_log.display()
                )
            })?;
        }

        Ok(())
    }

    fn set_worker_state(
        &self,
        project_id: &str,
        state: WorkerState,
        last_error: Option<String>,
    ) -> Result<WorkerSummary> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| anyhow!("failed to acquire worker state lock"))?;
        let runtime = workers
            .get_mut(project_id)
            .ok_or_else(|| anyhow!("worker {project_id} not found"))?;
        runtime.summary.status = state.clone();
        runtime.last_error = last_error.clone();
        runtime.indexer_state.state = state.clone();
        if state != WorkerState::Indexing {
            runtime.indexer_state.progress_percent = None;
            runtime.indexer_state.current_file = None;
        }
        let snapshot = runtime.snapshot();
        drop(workers);

        self.update_project_state(project_id, worker_state_label(&state), last_error)?;
        self.persist_worker_state(project_id)?;

        if state == WorkerState::Running {
            let mut registry = self
                .registry
                .lock()
                .map_err(|_| anyhow!("failed to acquire registry lock"))?;
            registry.mark_running(project_id, true)?;
        }

        Ok(snapshot)
    }

    fn update_index_progress(&self, project_id: &str, progress: IndexProgress) -> Result<()> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| anyhow!("failed to acquire worker state lock"))?;
        let runtime = workers
            .get_mut(project_id)
            .ok_or_else(|| anyhow!("worker {project_id} not found"))?;
        runtime.indexer_state.state = WorkerState::Indexing;
        runtime.indexer_state.progress_percent = progress.percent();
        runtime.indexer_state.current_file = progress.current_file.clone();
        runtime.summary.indexed_files_count = progress.files_completed as u64;
        drop(workers);

        self.update_project_state(project_id, "indexing", None)?;
        self.persist_worker_state(project_id)
    }

    fn apply_index_report(&self, project_id: &str, report: &IndexReport) -> Result<()> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| anyhow!("failed to acquire worker state lock"))?;
        let runtime = workers
            .get_mut(project_id)
            .ok_or_else(|| anyhow!("worker {project_id} not found"))?;
        runtime.summary.indexed_files_count = report.files_total as u64;
        runtime.indexer_state.progress_percent = Some(100.0);
        runtime.indexer_state.current_file = None;
        drop(workers);

        self.persist_worker_state(project_id)
    }

    fn persist_worker_state(&self, project_id: &str) -> Result<()> {
        let snapshot = {
            let workers = self
                .workers
                .lock()
                .map_err(|_| anyhow!("failed to acquire worker state lock"))?;
            let runtime = workers
                .get(project_id)
                .ok_or_else(|| anyhow!("worker {project_id} not found"))?;
            WorkerStatePersist {
                summary: runtime.snapshot(),
                last_error: runtime.last_error.clone(),
                indexer_state: runtime.indexer_state.clone(),
            }
        };

        let worker_dir = self.runtime_dir.join("projects").join(project_id);
        fs::create_dir_all(&worker_dir)
            .with_context(|| format!("failed to create worker dir {}", worker_dir.display()))?;
        let state_file = worker_dir.join("worker-state.json");
        let content = serde_json::to_vec_pretty(&snapshot)?;
        fs::write(&state_file, content)
            .with_context(|| format!("failed to persist worker state to {}", state_file.display()))
    }

    fn worker_status(&self, project_id: &str) -> Option<WorkerStatus> {
        let (summary, indexer_state, is_watching) = {
            let runtime = self.runtime.lock().ok()?;
            let is_watching = runtime
                .get(project_id)
                .map(|entry| entry.info.watch)
                .unwrap_or(false);
            drop(runtime);
            let workers = self.workers.lock().ok()?;
            let worker = workers.get(project_id)?;
            (worker.snapshot(), worker.indexer_state.clone(), is_watching)
        };

        Some(WorkerStatus {
            indexer_state,
            summary,
            task_queue_size: 0,
            is_watching,
        })
    }

    fn master_status(&self) -> Result<MasterStatus> {
        let workers = {
            let guard = self
                .workers
                .lock()
                .map_err(|_| anyhow!("failed to acquire worker state lock"))?;
            guard.values().map(|w| w.snapshot()).collect()
        };

        Ok(MasterStatus {
            pid: std::process::id(),
            uptime_secs: 0,
            workers,
        })
    }

    fn resolve_project(&self, project: &ProjectRef) -> Option<RegistryEntry> {
        let registry = self.registry.lock().ok()?;
        match project {
            ProjectRef::Path(path) => fs::canonicalize(path).ok().and_then(|canonical| {
                registry
                    .contains_root(&canonical)
                    .map(|entry| entry.clone())
            }),
            ProjectRef::Id(id) => registry.by_id(id).cloned(),
        }
    }

    fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        let registry_entries = {
            let registry = self
                .registry
                .lock()
                .map_err(|_| anyhow!("failed to acquire registry lock"))?;
            registry.all()
        };

        registry_entries
            .iter()
            .map(|entry| self.project_info_for(entry))
            .collect()
    }

    fn project_status(&self, project: &ProjectRef) -> Result<ProjectInfo> {
        let entry = self
            .resolve_project(project)
            .ok_or_else(|| anyhow!("project not found"))?;
        self.project_info_for(&entry)
    }

    fn restart_projects(&self, projects: Vec<ProjectRef>) -> Result<Vec<ProjectInfo>> {
        let mut restarted = Vec::new();
        for project in projects {
            let entry = match self.resolve_project(&project) {
                Some(entry) => entry,
                None => bail!("project not found"),
            };

            self.update_project_state(&entry.project_id, "restarting", None)?;
            {
                let mut workers = self
                    .workers
                    .lock()
                    .map_err(|_| anyhow!("failed to acquire worker state lock"))?;
                workers.remove(&entry.project_id);
            }

            let request = ProjectAddRequest {
                project_root: entry.project_root.clone(),
                id: Some(entry.project_id.clone()),
                autostart: entry.autostart,
                watch: entry.watch,
                index_mode: entry.index_mode.clone(),
                model: entry.model.clone(),
            };
            self.spawn_worker(entry.clone(), request)?;
            restarted.push(self.project_info_for(&entry)?);
        }
        Ok(restarted)
    }

    fn project_info_for(&self, entry: &RegistryEntry) -> Result<ProjectInfo> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| anyhow!("failed to acquire runtime state lock"))?;
        if let Some(existing) = runtime.get(&entry.project_id) {
            return Ok(existing.info.clone());
        }

        let info = ProjectInfo {
            project_id: entry.project_id.clone(),
            project_root: entry.project_root.clone(),
            autostart: entry.autostart,
            watch: entry.watch,
            index_mode: entry.index_mode.clone(),
            model: entry.model.clone(),
            state: entry
                .last_state
                .clone()
                .unwrap_or_else(|| "pending".to_string()),
        };

        runtime.insert(
            entry.project_id.clone(),
            ProjectRuntimeEntry {
                info: info.clone(),
                last_error: None,
            },
        );

        Ok(info)
    }
}

fn worker_state_label(state: &WorkerState) -> &'static str {
    match state {
        WorkerState::Running | WorkerState::Idle | WorkerState::Indexing => "running",
        WorkerState::Paused => "paused",
        WorkerState::Failed => "failed",
        WorkerState::Starting => "starting",
        WorkerState::Stopping => "stopping",
    }
}

fn generate_project_id(path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[derive(Debug, Clone, Deserialize)]
struct PartialListRequest {
    filter: Option<String>,
    limit: Option<u32>,
}

#[derive(Clone, Debug)]
struct ProjectRuntimeEntry {
    info: ProjectInfo,
    last_error: Option<String>,
}

#[derive(Debug)]
struct WorkerRuntimeEntry {
    summary: WorkerSummary,
    started_at: Instant,
    last_error: Option<String>,
    indexer_state: IndexerState,
}

impl WorkerRuntimeEntry {
    fn snapshot(&self) -> WorkerSummary {
        let mut summary = self.summary.clone();
        summary.uptime_secs = self.started_at.elapsed().as_secs();
        summary
    }
}

#[derive(Debug, Serialize)]
struct WorkerStatePersist {
    summary: WorkerSummary,
    last_error: Option<String>,
    indexer_state: IndexerState,
}

fn print_start_feedback(config: &MasterConfig, registry: &ProjectRegistry) {
    println!("code-navd master running");
    println!("  pid: {}", std::process::id());
    for socket in &config.listen {
        println!("  socket: {}", socket);
    }
    println!("  projects registered: {}", registry.project_count());
    println!("  runtime dir: {}", config.runtime_dir.display());
}

fn confirm_stop() -> Result<bool> {
    if !io::stdin().is_terminal() {
        bail!("当前为非交互式环境，请使用 --yes 确认停止");
    }
    print!("确定要停止 code-navd? [y/N]: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let decision = input.trim().to_ascii_lowercase();
    Ok(matches!(decision.as_str(), "y" | "yes"))
}

fn apply_autostart(config: &MasterConfig, registry: &ProjectRegistry) -> Result<()> {
    match config.autostart {
        AutostartMode::None => {
            info!("autostart disabled; no workers will be launched automatically");
        }
        AutostartMode::All => {
            let total = registry
                .entries
                .values()
                .filter(|entry| entry.last_running)
                .count();
            info!(
                count = total,
                "autostart queue prepared (TODO: spawn workers)"
            );
        }
        AutostartMode::List => {
            info!(
                count = config.autostart_list.len(),
                "autostart for specific projects (TODO: spawn workers)"
            );
        }
        AutostartMode::OnDemand => {
            info!("autostart on-demand; workers will start when requests arrive (TODO)");
        }
    }
    Ok(())
}

#[derive(Debug)]
struct LoggingGuards {
    _file_guard: Option<WorkerGuard>,
}

impl LoggingGuards {
    fn init(config: &MasterConfig) -> Result<Self> {
        use tracing_subscriber::{fmt, prelude::*, registry};

        let logs_service = logs::init_global(config.log_history_limit);
        let capture_layer = logs::CaptureLayer::new(logs_service, "master");
        let console_layer = fmt::layer()
            .with_target(true)
            .with_ansi(config.foreground)
            .with_timer(UtcTime::rfc_3339())
            .with_writer(|| std::io::stdout())
            .with_filter(LevelFilter::from_level(config.log_level));

        let base = registry().with(capture_layer).with(console_layer);

        if let Some(file_path) = &config.log_file {
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create log directory {}", parent.display())
                })?;
            }
            let directory = file_path.parent().unwrap_or_else(|| Path::new("."));
            let file_name = file_path
                .file_name()
                .unwrap_or_else(|| OsStr::new("master.log"))
                .to_string_lossy()
                .into_owned();
            let file = tracing_appender::rolling::never(directory, file_name);
            let (non_blocking, worker_guard) = tracing_appender::non_blocking(file);
            let file_layer = fmt::layer()
                .json()
                .with_writer(non_blocking)
                .with_timer(UtcTime::rfc_3339())
                .with_target(true)
                .with_filter(LevelFilter::from_level(config.log_level));
            base.with(file_layer)
                .try_init()
                .map_err(|err| anyhow!("failed to initialize logging: {err}"))?;
            return Ok(Self {
                _file_guard: Some(worker_guard),
            });
        }

        base.try_init()
            .map_err(|err| anyhow!("failed to initialize logging: {err}"))?;

        Ok(Self { _file_guard: None })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RegistryEntry {
    project_id: String,
    project_root: PathBuf,
    #[serde(default)]
    autostart: bool,
    #[serde(default)]
    watch: bool,
    #[serde(default = "default_index_mode")]
    index_mode: IndexMode,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    last_running: bool,
    #[serde(default)]
    last_state: Option<String>,
}

fn default_index_mode() -> IndexMode {
    IndexMode::Auto
}

#[derive(Debug)]
struct ProjectRegistry {
    path: PathBuf,
    entries: BTreeMap<String, RegistryEntry>,
}

impl ProjectRegistry {
    fn load(config: &MasterConfig) -> Result<Self> {
        let projects_dir = config.runtime_dir.join("projects");
        fs::create_dir_all(&projects_dir)
            .with_context(|| format!("failed to create {}", projects_dir.display()))?;
        let path = projects_dir.join("registry.json");
        if !path.exists() {
            let file = RegistryFile {
                version: 1,
                projects: Vec::new(),
            };
            let json = serde_json::to_vec_pretty(&file)?;
            fs::write(&path, json)?;
        }

        let content = fs::read(&path)?;
        let parsed: RegistryFile =
            serde_json::from_slice(&content).context("invalid registry.json")?;
        let mut map = BTreeMap::new();
        for entry in parsed.projects {
            map.insert(entry.project_root.to_string_lossy().to_string(), entry);
        }
        Ok(Self { path, entries: map })
    }

    fn project_count(&self) -> usize {
        self.entries.len()
    }

    fn contains_root(&self, path: &Path) -> Option<&RegistryEntry> {
        let key = path.to_string_lossy();
        self.entries.get(key.as_ref())
    }

    fn contains_root_mut(&mut self, path: &Path) -> Option<&mut RegistryEntry> {
        let key = path.to_string_lossy();
        self.entries.get_mut(key.as_ref())
    }

    fn contains_id(&self, project_id: &str) -> Option<&RegistryEntry> {
        self.entries
            .values()
            .find(|entry| entry.project_id == project_id)
    }

    fn by_id(&self, project_id: &str) -> Option<&RegistryEntry> {
        self.entries
            .values()
            .find(|entry| entry.project_id == project_id)
    }

    fn by_id_mut(&mut self, project_id: &str) -> Option<&mut RegistryEntry> {
        self.entries
            .values_mut()
            .find(|entry| entry.project_id == project_id)
    }

    fn all(&self) -> Vec<RegistryEntry> {
        self.entries.values().cloned().collect()
    }

    fn mark_running(&mut self, project_id: &str, running: bool) -> Result<()> {
        if let Some(entry) = self.by_id_mut(project_id) {
            entry.last_running = running;
            return self.persist();
        }
        bail!("project {project_id} not found in registry")
    }

    fn add(&mut self, entry: RegistryEntry) -> Result<()> {
        let key = entry.project_root.to_string_lossy().to_string();
        self.entries.insert(key, entry);
        self.persist()
    }

    fn persist(&self) -> Result<()> {
        let file = RegistryFile {
            version: 1,
            projects: self.entries.values().cloned().collect(),
        };
        let json = serde_json::to_vec_pretty(&file)?;
        fs::write(&self.path, json)
            .with_context(|| format!("failed to write registry file {}", self.path.display()))
    }
}

#[derive(Serialize, Deserialize)]
struct RegistryFile {
    version: u32,
    projects: Vec<RegistryEntry>,
}

#[derive(Debug, Clone)]
struct MasterConfig {
    runtime_dir: PathBuf,
    listen: Vec<SocketAddrKind>,
    foreground: bool,
    grace_period: Duration,
    log_dir: PathBuf,
    worker_log_spool: PathBuf,
    log_level: Level,
    log_file: Option<PathBuf>,
    log_history_limit: usize,
    autostart: AutostartMode,
    autostart_list: Vec<PathBuf>,
    config_path: Option<PathBuf>,
}

#[derive(Debug)]
struct StopConfig {
    runtime_dir: PathBuf,
    listen: Vec<SocketAddrKind>,
    pid_path: PathBuf,
    lock_path: PathBuf,
    grace: Duration,
    timeout: Duration,
    force: bool,
    assume_yes: bool,
}

impl MasterConfig {
    fn load(args: &StartCommand) -> Result<Self> {
        let defaults = RawConfig::default_paths()?;
        let mut merged = defaults.clone();

        let mut config_candidates = Vec::new();
        if let Some(cli_path) = args.config_path.clone() {
            config_candidates.push(cli_path);
        }
        if let Some(env_path) = config_path_from_env() {
            config_candidates.push(env_path);
        }
        if let Some(default_path) = defaults.config_path {
            config_candidates.push(default_path);
        }

        for config_path in config_candidates {
            if config_path.exists() {
                let file = RawConfig::from_file(&config_path)
                    .with_context(|| format!("failed to read config {}", config_path.display()))?;
                merged = merged.merge(file).with_config_path(config_path);
                break;
            }
        }

        let env_cfg = RawConfig::from_env()?;
        merged = merged.merge(env_cfg);

        let mut cli_cfg = RawConfig::default();
        cli_cfg.runtime_dir = args.runtime_dir.clone();
        cli_cfg.listen = args.socket.clone().map(|s| vec![s]);
        if args.foreground {
            cli_cfg.foreground = Some(true);
        }
        cli_cfg.grace_period_secs = args.grace;
        if let Some(level) = args.log_level {
            cli_cfg.log = Some(RawLogConfig {
                level: Some(level),
                dir: args.log_dir.clone(),
                worker_spool_dir: args.worker_log_spool.clone(),
                file: None,
                history_limit: None,
            });
        }
        if args.log_dir.is_some() || args.worker_log_spool.is_some() {
            let mut log_cfg = cli_cfg.log.take().unwrap_or_default();
            if args.log_dir.is_some() {
                log_cfg.dir = args.log_dir.clone();
            }
            if args.worker_log_spool.is_some() {
                log_cfg.worker_spool_dir = args.worker_log_spool.clone();
            }
            cli_cfg.log = Some(log_cfg);
        }
        cli_cfg.autostart = Some(RawAutostartConfig {
            mode: Some(args.autostart),
            projects: None,
        });
        merged = merged.merge(cli_cfg);

        merged.build()
    }
}

impl StopConfig {
    fn load(args: &StopCommand) -> Result<Self> {
        let defaults = RawConfig::default_paths()?;
        let mut merged = defaults.clone();

        let mut config_candidates = Vec::new();
        if let Some(cli_path) = args.config_path.clone() {
            config_candidates.push(cli_path);
        }
        if let Some(env_path) = config_path_from_env() {
            config_candidates.push(env_path);
        }
        if let Some(default_path) = defaults.config_path {
            config_candidates.push(default_path);
        }

        for config_path in config_candidates {
            if config_path.exists() {
                let file = RawConfig::from_file(&config_path)
                    .with_context(|| format!("failed to read config {}", config_path.display()))?;
                merged = merged.merge(file).with_config_path(config_path);
                break;
            }
        }

        let env_cfg = RawConfig::from_env()?;
        merged = merged.merge(env_cfg);

        if let Some(socket) = args.socket.clone() {
            merged.listen = Some(vec![socket]);
        }

        let runtime_dir = merged.runtime_dir.unwrap_or(default_runtime_dir()?);
        let listen = parse_listen(merged.listen, &runtime_dir)?;
        let grace = Duration::from_secs(args.grace_secs.clamp(1, 600));
        let timeout_secs = args.timeout_secs.max(args.grace_secs).clamp(1, 1800);
        let timeout = Duration::from_secs(timeout_secs);

        Ok(Self {
            runtime_dir: runtime_dir.clone(),
            listen,
            pid_path: runtime_dir.join("master.pid"),
            lock_path: runtime_dir.join("master.lock"),
            grace,
            timeout,
            force: args.force,
            assume_yes: args.assume_yes,
        })
    }
}

fn cleanup_runtime_artifacts(config: &StopConfig) -> Result<()> {
    if config.pid_path.exists() {
        let _ = fs::remove_file(&config.pid_path);
    }
    if config.lock_path.exists() {
        let _ = fs::remove_file(&config.lock_path);
    }
    for socket_addr_kind in &config.listen {
        if let SocketAddrKind::Unix(path) = socket_addr_kind {
            if path.exists() {
                let _ = fs::remove_file(path);
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct RawConfig {
    runtime_dir: Option<PathBuf>,
    listen: Option<Vec<String>>,
    foreground: Option<bool>,
    grace_period_secs: Option<u64>,
    log: Option<RawLogConfig>,
    autostart: Option<RawAutostartConfig>,
    config_path: Option<PathBuf>,
}

impl RawConfig {
    fn default_paths() -> Result<Self> {
        let runtime_dir = default_runtime_dir()?;
        Ok(Self {
            runtime_dir: Some(runtime_dir),
            listen: None,
            foreground: None,
            grace_period_secs: None,
            log: Some(RawLogConfig::default()),
            autostart: Some(RawAutostartConfig {
                mode: Some(AutostartMode::All),
                projects: None,
            }),
            config_path: default_config_path(),
        })
    }

    fn from_file(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let parsed: FileConfig = toml::from_str(&content)?;
        Ok(parsed.into())
    }

    fn from_env() -> Result<Self> {
        let mut cfg = Self::default();
        if let Ok(runtime) = std::env::var("CODE_NAV_MASTER_RUNTIME_DIR") {
            cfg.runtime_dir = Some(PathBuf::from(runtime));
        }
        if let Ok(socket) = std::env::var("CODE_NAV_MASTER_SOCKET") {
            cfg.listen = Some(vec![socket]);
        }
        if let Ok(level) = std::env::var("CODE_NAV_MASTER_LOG_LEVEL") {
            if let Ok(parsed) = LogLevel::from_str_case_insensitive(&level) {
                cfg.log = Some(RawLogConfig {
                    level: Some(parsed),
                    dir: None,
                    worker_spool_dir: None,
                    file: None,
                    history_limit: None,
                });
            }
        }
        if let Ok(dir) = std::env::var("CODE_NAV_MASTER_LOG_DIR") {
            let mut log_cfg = cfg.log.take().unwrap_or_default();
            log_cfg.dir = Some(PathBuf::from(dir));
            cfg.log = Some(log_cfg);
        }
        if let Ok(spool_dir) = std::env::var("CODE_NAV_MASTER_WORKER_LOG_SPOOL") {
            let mut log_cfg = cfg.log.take().unwrap_or_default();
            log_cfg.worker_spool_dir = Some(PathBuf::from(spool_dir));
            cfg.log = Some(log_cfg);
        }
        if let Ok(grace) = std::env::var("CODE_NAV_MASTER_GRACE") {
            if let Ok(value) = grace.parse() {
                cfg.grace_period_secs = Some(value);
            }
        }
        Ok(cfg)
    }

    fn merge(mut self, other: Self) -> Self {
        if other.runtime_dir.is_some() {
            self.runtime_dir = other.runtime_dir;
        }
        if other.listen.is_some() {
            self.listen = other.listen;
        }
        if other.foreground.is_some() {
            self.foreground = other.foreground;
        }
        if other.grace_period_secs.is_some() {
            self.grace_period_secs = other.grace_period_secs;
        }
        if let Some(log) = other.log {
            self.log = Some(match self.log.take() {
                Some(current) => current.merge(log),
                None => log,
            });
        }
        if let Some(autostart) = other.autostart {
            self.autostart = Some(match self.autostart.take() {
                Some(current) => current.merge(autostart),
                None => autostart,
            });
        }
        if other.config_path.is_some() {
            self.config_path = other.config_path;
        }
        self
    }

    fn with_config_path(mut self, path: PathBuf) -> Self {
        self.config_path = Some(path);
        self
    }

    fn build(self) -> Result<MasterConfig> {
        let runtime_dir = self.runtime_dir.unwrap_or(default_runtime_dir()?);
        fs::create_dir_all(&runtime_dir)
            .with_context(|| format!("failed to create runtime dir {}", runtime_dir.display()))?;

        let listen = parse_listen(self.listen, &runtime_dir)?;
        let foreground = self.foreground.unwrap_or(false);
        let grace_secs = self.grace_period_secs.unwrap_or(15).clamp(1, 300);
        let log_cfg = self.log.unwrap_or_default();
        let log_level = log_cfg.level.map(LogLevel::to_level).unwrap_or(Level::INFO);
        let log_dir = log_cfg.dir.unwrap_or_else(|| runtime_dir.join("logs"));
        let log_dir = if log_dir.is_relative() {
            runtime_dir.join(log_dir)
        } else {
            log_dir
        };
        fs::create_dir_all(&log_dir)
            .with_context(|| format!("failed to create log dir {}", log_dir.display()))?;

        let worker_log_spool = log_cfg
            .worker_spool_dir
            .unwrap_or_else(|| runtime_dir.join("worker-log-spool"));
        let worker_log_spool = if worker_log_spool.is_relative() {
            runtime_dir.join(worker_log_spool)
        } else {
            worker_log_spool
        };
        fs::create_dir_all(&worker_log_spool).with_context(|| {
            format!(
                "failed to create worker log spool dir {}",
                worker_log_spool.display()
            )
        })?;

        let log_file = log_cfg.file.map(|mut path| {
            if path.is_relative() {
                path = runtime_dir.join(path);
            }
            path
        });
        let log_file = match (log_file, foreground) {
            (Some(path), _) => Some(path),
            (None, false) => Some(log_dir.join("master.log")),
            (None, true) => None,
        };

        let log_history_limit = logs::history_capacity_or_default(log_cfg.history_limit);

        let (autostart, list) = self
            .autostart
            .map(|cfg| {
                (
                    cfg.mode.unwrap_or(AutostartMode::All),
                    cfg.projects.unwrap_or_default(),
                )
            })
            .unwrap_or((AutostartMode::All, Vec::new()));
        let autostart_list = list
            .into_iter()
            .map(|path| {
                if path.is_relative() {
                    runtime_dir.join(path)
                } else {
                    path
                }
            })
            .collect();

        Ok(MasterConfig {
            runtime_dir,
            listen,
            foreground,
            grace_period: Duration::from_secs(grace_secs),
            log_dir,
            worker_log_spool,
            log_level,
            log_file,
            log_history_limit,
            autostart,
            autostart_list,
            config_path: self.config_path,
        })
    }
}

impl Default for RawConfig {
    fn default() -> Self {
        Self {
            runtime_dir: None,
            listen: None,
            foreground: None,
            grace_period_secs: None,
            log: None,
            autostart: None,
            config_path: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct FileConfig {
    runtime_dir: Option<PathBuf>,
    listen: Option<Vec<String>>,
    foreground: Option<bool>,
    grace_period_secs: Option<u64>,
    log: Option<RawLogConfig>,
    autostart: Option<RawAutostartConfig>,
}

impl From<FileConfig> for RawConfig {
    fn from(value: FileConfig) -> Self {
        Self {
            runtime_dir: value.runtime_dir,
            listen: value.listen,
            foreground: value.foreground,
            grace_period_secs: value.grace_period_secs,
            log: value.log,
            autostart: value.autostart,
            config_path: None,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawLogConfig {
    level: Option<LogLevel>,
    dir: Option<PathBuf>,
    worker_spool_dir: Option<PathBuf>,
    file: Option<PathBuf>,
    history_limit: Option<usize>,
}

impl RawLogConfig {
    fn merge(mut self, other: RawLogConfig) -> Self {
        if other.level.is_some() {
            self.level = other.level;
        }
        if other.dir.is_some() {
            self.dir = other.dir;
        }
        if other.worker_spool_dir.is_some() {
            self.worker_spool_dir = other.worker_spool_dir;
        }
        if other.file.is_some() {
            self.file = other.file;
        }
        if other.history_limit.is_some() {
            self.history_limit = other.history_limit;
        }
        self
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RawAutostartConfig {
    mode: Option<AutostartMode>,
    projects: Option<Vec<PathBuf>>,
}

impl RawAutostartConfig {
    fn merge(mut self, other: RawAutostartConfig) -> Self {
        if other.mode.is_some() {
            self.mode = other.mode;
        }
        if other.projects.is_some() {
            self.projects = other.projects;
        }
        self
    }
}

fn config_path_from_env() -> Option<PathBuf> {
    std::env::var("CODE_NAV_MASTER_CONFIG")
        .ok()
        .map(PathBuf::from)
}

fn default_config_path() -> Option<PathBuf> {
    default_config_dir().map(|dir| dir.join("master.toml"))
}

fn default_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("code-nav"))
}

fn default_runtime_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("unable to determine home directory"))?;
    Ok(home.join(".code-nav"))
}

fn parse_listen(uris: Option<Vec<String>>, runtime_dir: &Path) -> Result<Vec<SocketAddrKind>> {
    let mut sockets = Vec::new();
    if let Some(uris) = uris {
        for uri in uris {
            sockets.push(parse_socket_value(&uri)?);
        }
    } else {
        // Default behavior if no listen URIs are provided
        if cfg!(windows) {
            sockets.push(SocketAddrKind::NamedPipe(String::from(
                r"\\.\pipe\code-nav-master",
            )));
        } else {
            sockets.push(SocketAddrKind::Unix(runtime_dir.join("master.sock")));
        }
        // Add default HTTP listener
        sockets.push(SocketAddrKind::Tcp("0.0.0.0:6688".parse().unwrap()));
    }
    Ok(sockets)
}

fn parse_socket_value(value: &str) -> Result<SocketAddrKind> {
    if let Some(rest) = value.strip_prefix("uds://") {
        let path = PathBuf::from(rest);
        return Ok(SocketAddrKind::Unix(path));
    }
    if let Some(rest) = value.strip_prefix("npipe://") {
        return Ok(SocketAddrKind::NamedPipe(rest.to_string()));
    }
    if let Some(rest) = value.strip_prefix("tcp://") {
        let mut addrs = rest
            .to_socket_addrs()
            .with_context(|| format!("invalid TCP socket {value}"))?;
        if let Some(addr) = addrs.next() {
            return Ok(SocketAddrKind::Tcp(addr));
        }
    }
    Err(anyhow!("unknown socket uri: {value}"))
}

#[derive(Debug)]
struct MasterLock {
    lock: LockFile,
    pid_path: PathBuf,
}

impl MasterLock {
    fn acquire(config: &MasterConfig) -> Result<Self> {
        let lock_path = config.runtime_dir.join("master.lock");
        let pid_path = config.runtime_dir.join("master.pid");
        let mut lock = LockFile::open(&lock_path)
            .with_context(|| format!("failed to open lock file {}", lock_path.display()))?;
        if let Err(err) = lock.try_lock() {
            if err.kind() == io::ErrorKind::WouldBlock {
                let info = read_pid_file(&pid_path).unwrap_or_default();
                bail!(
                    "code-navd is already running (pid: {:?}, socket: {:?})",
                    info.pid,
                    info.socket
                );
            }
            return Err(err).with_context(|| format!("failed to lock {}", lock_path.display()));
        }

        write_pid_file(&pid_path, &config.listen)?;
        Ok(Self { lock, pid_path })
    }
}

impl Drop for MasterLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.pid_path);
        let _ = self.lock.unlock();
    }
}

#[derive(Default)]
struct PidInfo {
    pid: Option<u32>,
    socket: Option<String>,
}

fn write_pid_file(pid_path: &Path, sockets: &[SocketAddrKind]) -> Result<()> {
    let socket_str = sockets
        .get(0)
        .map_or("unknown".to_string(), |s| s.to_string());
    let info = json!({
        "pid": std::process::id(),
        "started_at": Utc::now().to_rfc3339(),
        "socket": socket_str,
    });
    let mut file = File::create(pid_path)?;
    file.write_all(info.to_string().as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn read_pid_file(path: &Path) -> Result<PidInfo> {
    let contents = fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&contents)?;

    let pid = value.get("pid").and_then(|v| v.as_u64()).map(|v| v as u32);
    let socket = value
        .get("socket")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    Ok(PidInfo { pid, socket })
}

#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    unsafe {
        let res = libc::kill(pid as libc::pid_t, 0);
        if res == 0 {
            true
        } else {
            match io::Error::last_os_error().raw_os_error() {
                Some(code) if code == libc::EPERM => true,
                Some(code) if code == libc::ESRCH => false,
                _ => false,
            }
        }
    }
}

#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SYNCHRONIZE,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_SYNCHRONIZE, 0, pid);
        if handle == 0 {
            return false;
        }
        let mut exit_code = 0;
        let ok = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        ok != 0 && exit_code == STILL_ACTIVE
    }
}

#[cfg(not(any(unix, windows)))]
fn is_process_alive(_pid: u32) -> bool {
    false
}

#[cfg(unix)]
fn send_signal(pid: u32, force: bool) -> Result<()> {
    let signo = if force { libc::SIGKILL } else { libc::SIGTERM };
    let res = unsafe { libc::kill(pid as libc::pid_t, signo) };
    if res == 0 {
        Ok(())
    } else {
        match io::Error::last_os_error().raw_os_error() {
            Some(code) if code == libc::ESRCH => Ok(()),
            _ => Err(io::Error::last_os_error()).context("无法发送信号"),
        }
    }
}

#[cfg(windows)]
fn send_signal(pid: u32, force: bool) -> Result<()> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    let _ = force;
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle == 0 {
            return Err(io::Error::last_os_error()).context("无法打开进程句柄");
        }
        let ok = TerminateProcess(handle, 0);
        CloseHandle(handle);
        if ok == 0 {
            Err(io::Error::last_os_error()).context("无法终止进程")
        } else {
            Ok(())
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn send_signal(pid: u32, force: bool) -> Result<()> {
    let _ = (pid, force);
    Err(anyhow!("当前平台不支持 stop 命令"))
}

#[derive(Clone, Debug)]
pub enum SocketAddrKind {
    Unix(PathBuf),
    NamedPipe(String),
    Tcp(SocketAddr),
}

impl std::fmt::Display for SocketAddrKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SocketAddrKind::Unix(path) => write!(f, "uds://{}", path.display()),
            SocketAddrKind::NamedPipe(name) => write!(f, "npipe://{name}"),
            SocketAddrKind::Tcp(addr) => write!(f, "tcp://{addr}"),
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[value(rename_all = "kebab-case")]
pub enum AutostartMode {
    None,
    All,
    List,
    #[serde(rename = "on-demand")]
    #[value(name = "on-demand")]
    OnDemand,
}

#[derive(Copy, Clone, Debug, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[value(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    fn from_str_case_insensitive(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "trace" => Ok(Self::Trace),
            "debug" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "warn" | "warning" => Ok(Self::Warn),
            "error" => Ok(Self::Error),
            other => Err(anyhow!("unknown log level {other}")),
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        };
        write!(f, "{text}")
    }
}

impl LogLevel {
    fn to_level(self) -> Level {
        match self {
            LogLevel::Trace => Level::TRACE,
            LogLevel::Debug => Level::DEBUG,
            LogLevel::Info => Level::INFO,
            LogLevel::Warn => Level::WARN,
            LogLevel::Error => Level::ERROR,
        }
    }
}
