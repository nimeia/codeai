use std::{
    collections::HashMap,
    ffi::OsStr,
    fs::{self, File},
    io::{self, Write},
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use cfg_if::cfg_if;
use chrono::Utc;
use clap::ValueEnum;
use fslock::LockFile;
use is_terminal::IsTerminal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{info, warn, Level};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::LevelFilter;

#[derive(Debug)]
pub struct StartCommand {
    pub config_path: Option<PathBuf>,
    pub runtime_dir: Option<PathBuf>,
    pub socket: Option<String>,
    pub log_level: Option<LogLevel>,
    pub foreground: bool,
    pub grace: Option<u64>,
    pub autostart: AutostartMode,
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

pub fn run_start(args: StartCommand) -> Result<()> {
    let config = MasterConfig::load(&args)?;
    let _lock = MasterLock::acquire(&config)?;
    let logging = LoggingGuards::init(&config)?;

    if !config.foreground {
        warn!("后台模式尚未实现，当前进程将以前台模式运行");
    }

    let endpoint = setup_control_endpoint(&config)?;
    let registry = ProjectRegistry::load(&config)?;
    apply_autostart(&config, &registry)?;

    print_start_feedback(&config, &registry);
    run_main_loop(&config, endpoint, logging)
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

pub fn run_restart(cmd: RestartCommand) -> Result<()> {
    println!("准备停止 code-navd...");
    run_stop(cmd.stop)?;
    println!("守护进程已停止，准备重新启动...");
    run_start(cmd.start)
}

fn run_main_loop(
    _config: &MasterConfig,
    endpoint: ControlEndpoint,
    _logging: LoggingGuards,
) -> Result<()> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let signal_flag = shutdown.clone();
    ctrlc::set_handler(move || {
        signal_flag.store(true, Ordering::SeqCst);
    })
    .context("failed to install ctrl-c handler")?;

    info!(
        socket = %endpoint,
        "code-nav master running; press Ctrl+C to exit"
    );

    while !shutdown.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_secs(1));
    }

    info!("shutdown requested, exiting master loop");
    Ok(())
}

fn print_start_feedback(config: &MasterConfig, registry: &ProjectRegistry) {
    println!("code-navd master running");
    println!("  pid: {}", std::process::id());
    println!("  socket: {}", config.socket);
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

fn setup_control_endpoint(config: &MasterConfig) -> Result<ControlEndpoint> {
    match &config.socket {
        SocketAddrKind::Unix(path) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "failed to create unix socket directory {}",
                        parent.display()
                    )
                })?;
            }
            if path.exists() {
                fs::remove_file(path).with_context(|| {
                    format!("failed to remove existing socket {}", path.display())
                })?;
            }
            info!(socket = %config.socket, "prepared unix socket (listener TODO)");
            Ok(ControlEndpoint::Unix(path.clone()))
        }
        SocketAddrKind::NamedPipe(name) => {
            info!(socket = %config.socket, "prepared named pipe (listener TODO)");
            Ok(ControlEndpoint::NamedPipe(name.clone()))
        }
        SocketAddrKind::Tcp(addr) => {
            info!(socket = %config.socket, "prepared TCP endpoint (listener TODO)");
            Ok(ControlEndpoint::Tcp(*addr))
        }
    }
}

#[derive(Debug)]
struct LoggingGuards {
    _file_guard: Option<WorkerGuard>,
}

impl LoggingGuards {
    fn init(config: &MasterConfig) -> Result<Self> {
        use tracing_subscriber::{fmt, prelude::*, registry};

        let console_layer = fmt::layer()
            .with_target(true)
            .with_ansi(config.foreground)
            .with_writer(|| std::io::stdout())
            .with_filter(LevelFilter::from_level(config.log_level));

        let base = registry().with(console_layer);

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
            base.with(
                fmt::layer()
                    .with_writer(non_blocking)
                    .with_target(true)
                    .with_ansi(false)
                    .with_filter(LevelFilter::from_level(config.log_level)),
            )
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
    project_root: PathBuf,
    last_running: bool,
    last_state: Option<String>,
}

#[derive(Debug)]
struct ProjectRegistry {
    path: PathBuf,
    entries: HashMap<String, RegistryEntry>,
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
        let mut map = HashMap::new();
        for entry in parsed.projects {
            map.insert(entry.project_root.to_string_lossy().to_string(), entry);
        }
        Ok(Self { path, entries: map })
    }

    fn project_count(&self) -> usize {
        self.entries.len()
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
    socket: SocketAddrKind,
    foreground: bool,
    grace_period: Duration,
    log_level: Level,
    log_file: Option<PathBuf>,
    autostart: AutostartMode,
    autostart_list: Vec<PathBuf>,
    config_path: Option<PathBuf>,
}

#[derive(Debug)]
struct StopConfig {
    runtime_dir: PathBuf,
    socket: SocketAddrKind,
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
        cli_cfg.socket = args.socket.clone();
        if args.foreground {
            cli_cfg.foreground = Some(true);
        }
        cli_cfg.grace_period_secs = args.grace;
        if let Some(level) = args.log_level {
            cli_cfg.log = Some(RawLogConfig {
                level: Some(level),
                file: None,
            });
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

        if let Some(runtime_dir) = args.runtime_dir.clone() {
            merged.runtime_dir = Some(runtime_dir);
        }
        if let Some(socket) = args.socket.clone() {
            merged.socket = Some(socket);
        }

        let runtime_dir = merged.runtime_dir.unwrap_or(default_runtime_dir()?);
        let socket = parse_socket(merged.socket.as_deref(), &runtime_dir)?;
        let grace = Duration::from_secs(args.grace_secs.clamp(1, 600));
        let timeout_secs = args.timeout_secs.max(args.grace_secs).clamp(1, 1800);
        let timeout = Duration::from_secs(timeout_secs);

        Ok(Self {
            runtime_dir: runtime_dir.clone(),
            socket,
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
    if let SocketAddrKind::Unix(path) = &config.socket {
        if path.exists() {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct RawConfig {
    runtime_dir: Option<PathBuf>,
    socket: Option<String>,
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
            socket: None,
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
            cfg.socket = Some(socket);
        }
        if let Ok(level) = std::env::var("CODE_NAV_MASTER_LOG_LEVEL") {
            if let Ok(parsed) = LogLevel::from_str_case_insensitive(&level) {
                cfg.log = Some(RawLogConfig {
                    level: Some(parsed),
                    file: None,
                });
            }
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
        if other.socket.is_some() {
            self.socket = other.socket;
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

        let socket = parse_socket(self.socket.as_deref(), &runtime_dir)?;
        let foreground = self.foreground.unwrap_or(false);
        let grace_secs = self.grace_period_secs.unwrap_or(15).clamp(1, 300);
        let log_cfg = self.log.unwrap_or_default();
        let log_level = log_cfg.level.map(Level::from).unwrap_or(Level::INFO);
        let log_file = log_cfg.file.map(|mut path| {
            if path.is_relative() {
                path = runtime_dir.join(path);
            }
            path
        });

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
            socket,
            foreground,
            grace_period: Duration::from_secs(grace_secs),
            log_level,
            log_file,
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
            socket: None,
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
    socket: Option<String>,
    foreground: Option<bool>,
    grace_period_secs: Option<u64>,
    log: Option<RawLogConfig>,
    autostart: Option<RawAutostartConfig>,
}

impl From<FileConfig> for RawConfig {
    fn from(value: FileConfig) -> Self {
        Self {
            runtime_dir: value.runtime_dir,
            socket: value.socket,
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
    file: Option<PathBuf>,
}

impl RawLogConfig {
    fn merge(mut self, other: RawLogConfig) -> Self {
        if other.level.is_some() {
            self.level = other.level;
        }
        if other.file.is_some() {
            self.file = other.file;
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

fn parse_socket(uri: Option<&str>, runtime_dir: &Path) -> Result<SocketAddrKind> {
    if let Some(value) = uri {
        parse_socket_value(value)
    } else if cfg!(windows) {
        Ok(SocketAddrKind::NamedPipe(String::from(
            r"\\.\pipe\code-nav-master",
        )))
    } else {
        Ok(SocketAddrKind::Unix(runtime_dir.join("master.sock")))
    }
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

        write_pid_file(&pid_path, &config.socket)?;
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

fn write_pid_file(pid_path: &Path, socket: &SocketAddrKind) -> Result<()> {
    let info = json!({
        "pid": std::process::id(),
        "started_at": Utc::now().to_rfc3339(),
        "socket": socket.to_string(),
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

fn is_process_alive(pid: u32) -> bool {
    cfg_if! {
        if #[cfg(unix)] {
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
        } else if #[cfg(windows)] {
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
        } else {
            false
        }
    }
}

fn send_signal(pid: u32, force: bool) -> Result<()> {
    cfg_if! {
        if #[cfg(unix)] {
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
        } else if #[cfg(windows)] {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

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
        } else {
            let _ = force;
            Err(anyhow!("当前平台不支持 stop 命令"))
        }
    }
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

#[derive(Clone, Debug)]
enum ControlEndpoint {
    Unix(PathBuf),
    NamedPipe(String),
    Tcp(SocketAddr),
}

impl std::fmt::Display for ControlEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ControlEndpoint::Unix(path) => write!(f, "uds://{}", path.display()),
            ControlEndpoint::NamedPipe(name) => write!(f, "npipe://{name}"),
            ControlEndpoint::Tcp(addr) => write!(f, "tcp://{addr}"),
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

impl From<LogLevel> for Level {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Trace => Level::TRACE,
            LogLevel::Debug => Level::DEBUG,
            LogLevel::Info => Level::INFO,
            LogLevel::Warn => Level::WARN,
            LogLevel::Error => Level::ERROR,
        }
    }
}
