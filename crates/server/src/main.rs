mod master;

use std::time::Duration;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

const READY_TIMEOUT_DEFAULT: u64 = 30;

#[derive(Parser)]
#[command(name = "code-navd", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 启动 code-nav master
    Start(StartArgs),
    /// 停止 code-nav master
    Stop(StopArgs),
    /// 重启 code-nav master
    Restart(RestartArgs),
    /// 其它子命令占位
    #[allow(dead_code)]
    #[command(hide = true)]
    Unknown,
}

#[derive(Args, Debug)]
struct StartArgs {
    /// 指定 master 配置文件路径
    #[arg(long)]
    config: Option<std::path::PathBuf>,
    /// 覆盖运行目录（默认 ~/.code-nav）
    #[arg(long)]
    runtime_dir: Option<std::path::PathBuf>,
    /// 覆盖控制端点 URI（uds://, npipe://, tcp://）
    #[arg(long)]
    socket: Option<String>,
    /// 设置日志级别
    #[arg(long, value_enum)]
    log_level: Option<master::LogLevel>,
    /// 前台运行（默认配置可覆盖）
    #[arg(long)]
    foreground: bool,
    /// 停机等待秒数
    #[arg(long)]
    grace: Option<u64>,
    /// 自动启动策略
    #[arg(long, value_enum, default_value = "all")]
    autostart: master::AutostartMode,
    /// 启动后等待控制端点 ready（默认开启）
    #[arg(long = "wait-ready", action = clap::ArgAction::SetTrue, conflicts_with = "no_wait")]
    wait_ready: bool,
    /// 启动后立即返回，不等待 ready
    #[arg(long = "no-wait", action = clap::ArgAction::SetTrue, conflicts_with = "wait_ready")]
    no_wait: bool,
    /// 等待 ready 的超时（秒）
    #[arg(long, default_value_t = READY_TIMEOUT_DEFAULT)]
    ready_timeout: u64,
}

#[derive(Args, Debug)]
struct StopArgs {
    /// 使用特定配置文件解析 runtime_dir/socket
    #[arg(long)]
    config: Option<std::path::PathBuf>,
    /// 指定运行目录（覆盖配置）
    #[arg(long)]
    runtime_dir: Option<std::path::PathBuf>,
    /// 覆盖控制端点 URI
    #[arg(long)]
    socket: Option<String>,
    /// 优雅等待时间（秒）
    #[arg(long, default_value_t = 10)]
    grace: u64,
    /// 总超时时间（秒）
    #[arg(long, default_value_t = 30)]
    timeout: u64,
    /// 超时后是否强制终止
    #[arg(long)]
    force: bool,
    /// 跳过确认提示
    #[arg(long)]
    yes: bool,
}

#[derive(Args, Debug)]
struct RestartArgs {
    /// 指定配置文件（同时用于 stop/start）
    #[arg(long)]
    config: Option<std::path::PathBuf>,
    /// 覆盖运行目录
    #[arg(long)]
    runtime_dir: Option<std::path::PathBuf>,
    /// 覆盖控制端点
    #[arg(long)]
    socket: Option<String>,
    /// start 时的日志级别
    #[arg(long, value_enum)]
    log_level: Option<master::LogLevel>,
    /// start 是否前台运行
    #[arg(long)]
    foreground: bool,
    /// start 的 grace (秒)
    #[arg(long)]
    start_grace: Option<u64>,
    /// start 的自动启动策略
    #[arg(long, value_enum, default_value = "all")]
    autostart: master::AutostartMode,
    /// stop 阶段宽限期
    #[arg(long, default_value_t = 10)]
    stop_grace: u64,
    /// stop 阶段超时时间
    #[arg(long, default_value_t = 30)]
    stop_timeout: u64,
    /// stop 是否允许强制
    #[arg(long)]
    stop_force: bool,
    /// 是否跳过确认（默认跳过）
    #[arg(long)]
    prompt: bool,
    /// restart 时 start 阶段是否等待 ready（默认开启）
    #[arg(long = "start-wait-ready", action = clap::ArgAction::SetTrue, conflicts_with = "start_no_wait")]
    start_wait_ready: bool,
    /// restart 时 start 阶段不等待 ready
    #[arg(long = "start-no-wait", action = clap::ArgAction::SetTrue, conflicts_with = "start_wait_ready")]
    start_no_wait: bool,
    /// restart 时等待 ready 的超时（秒）
    #[arg(long = "start-ready-timeout", default_value_t = READY_TIMEOUT_DEFAULT)]
    start_ready_timeout: u64,
}

impl StartArgs {
    fn wait_ready(&self) -> bool {
        if self.no_wait {
            false
        } else if self.wait_ready {
            true
        } else {
            true
        }
    }

    fn ready_timeout(&self) -> Duration {
        Duration::from_secs(self.ready_timeout.clamp(1, 600))
    }
}

impl RestartArgs {
    fn start_wait_ready(&self) -> bool {
        if self.start_no_wait {
            false
        } else if self.start_wait_ready {
            true
        } else {
            true
        }
    }

    fn start_ready_timeout(&self) -> Duration {
        Duration::from_secs(self.start_ready_timeout.clamp(1, 600))
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Start(args) => {
            let wait_ready = args.wait_ready();
            let ready_timeout = args.ready_timeout();
            let start = master::StartCommand {
                config_path: args.config,
                runtime_dir: args.runtime_dir,
                socket: args.socket,
                log_level: args.log_level,
                foreground: args.foreground,
                grace: args.grace,
                autostart: args.autostart,
                wait_ready,
                ready_timeout,
            };
            master::run_start(start)
        }
        Command::Stop(args) => {
            let stop = master::StopCommand {
                config_path: args.config,
                runtime_dir: args.runtime_dir,
                socket: args.socket,
                grace_secs: args.grace,
                timeout_secs: args.timeout,
                force: args.force,
                assume_yes: args.yes,
            };
            master::run_stop(stop)
        }
        Command::Restart(args) => {
            let start_wait_ready = args.start_wait_ready();
            let start_ready_timeout = args.start_ready_timeout();
            let start = master::StartCommand {
                config_path: args.config.clone(),
                runtime_dir: args.runtime_dir.clone(),
                socket: args.socket.clone(),
                log_level: args.log_level,
                foreground: args.foreground,
                grace: args.start_grace,
                autostart: args.autostart,
                wait_ready: start_wait_ready,
                ready_timeout: start_ready_timeout,
            };
            let stop = master::StopCommand {
                config_path: args.config,
                runtime_dir: args.runtime_dir,
                socket: args.socket,
                grace_secs: args.stop_grace,
                timeout_secs: args.stop_timeout,
                force: args.stop_force,
                assume_yes: !args.prompt,
            };
            master::run_restart(master::RestartCommand { start, stop })
        }
        Command::Unknown => Ok(()),
    }
}
