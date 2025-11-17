mod client;
mod commands;
mod formatter;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use code_nav_protocol::{
    InfoRequest, InfoResponse, ListKind, MasterInfo, MasterStatus, ProjectRef, Request,
    StatusRequest, StatusResponse, WorkerInfo, WorkerSummary,
};
use commands::{
    logs::{self, LogsArgs},
    project::{self, ProjectCommand},
};
use std::path::Path;

#[derive(Parser)]
#[command(name = "code-nav", version, about = "code navigation cli")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Search {
        query: String,
        #[arg(default_value_t = 5)]
        top_k: u32,
    },
    List {
        #[arg(value_enum)]
        kind: Kind,
    },
    Project {
        #[command(subcommand)]
        action: ProjectCommand,
    },
    Logs(LogsArgs),
    /// Show information about the daemon or a specific project
    Info(InfoArgs),
    /// Show status of the daemon and all project workers
    Status(StatusArgs),
}

#[derive(Debug, Args)]
pub struct InfoArgs {
    /// Get information for a specific project
    #[arg(long = "project", value_name = "PATH|ID")]
    pub project: Option<String>,
    /// Output in JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Output in JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::ValueEnum, Clone)]
enum Kind {
    Classes,
    Methods,
    Files,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::try_init().ok();
    let cli = Cli::parse();
    match cli.command {
        Commands::Search { query, top_k } => {
            let request = Request::Search(code_nav_protocol::SearchRequest { query, top_k });
            // TODO: send request to server; placeholder prints JSON.
            println!("{}", serde_json::to_string(&request)?);
        }
        Commands::List { kind } => {
            let list_kind = match kind {
                Kind::Classes => ListKind::Classes,
                Kind::Methods => ListKind::Methods,
                Kind::Files => ListKind::Files,
            };
            let request = Request::List(code_nav_protocol::ListRequest {
                kind: list_kind,
                filter: None,
                limit: None,
            });
            // TODO: send request to server; placeholder prints JSON.
            println!("{}", serde_json::to_string(&request)?);
        }
        Commands::Project { action } => {
            project::run(action)?;
        }
        Commands::Logs(args) => {
            logs::run(args)?;
        }
        Commands::Info(args) => {
            handle_info(args)?;
        }
        Commands::Status(args) => {
            handle_status(args)?;
        }
    }
    Ok(())
}

fn handle_info(args: InfoArgs) -> Result<()> {
    let target = args.project.map(|p| {
        if Path::new(&p).exists() {
            ProjectRef::Path(p)
        } else {
            ProjectRef::Id(p)
        }
    });

    let request = Request::Info(InfoRequest { target });
    let payload = serde_json::to_string(&request)?;

    if let Some(response_payload) = client::send_request(&payload)? {
        let response: code_nav_protocol::Response = serde_json::from_str(&response_payload)
            .context("failed to deserialize response from server")?;

        if let code_nav_protocol::Response::Info(info_response) = response {
            if args.json {
                formatter::print_line(&serde_json::to_string_pretty(&info_response)?);
            } else {
                print_info_response(info_response);
            }
        } else {
            formatter::print_line("Error: received unexpected response type from server");
        }
    }
    Ok(())
}

fn handle_status(args: StatusArgs) -> Result<()> {
    let request = Request::Status(StatusRequest { target: None });
    let payload = serde_json::to_string(&request)?;

    if let Some(response_payload) = client::send_request(&payload)? {
        let response: code_nav_protocol::Response = serde_json::from_str(&response_payload)
            .context("failed to deserialize response from server")?;

        if let code_nav_protocol::Response::Status(StatusResponse::Master(master_status)) = response
        {
            if args.json {
                formatter::print_line(&serde_json::to_string_pretty(&master_status)?);
            } else {
                print_master_status(master_status);
            }
        } else {
            formatter::print_line("Error: received unexpected response type from server");
        }
    }
    Ok(())
}

fn print_info_response(response: InfoResponse) {
    match response {
        InfoResponse::Master(info) => print_master_info(info),
        InfoResponse::Worker(info) => print_worker_info(info),
    }
}

fn print_master_info(info: MasterInfo) {
    formatter::print_line("CodeNav Master");
    formatter::print_line(&format!("{:<16} {}", "Version:", info.server_version));
    formatter::print_line(&format!("{:<16} {}", "Protocol:", info.protocol_version));
    formatter::print_line(&format!("{:<16} {}", "PID:", info.pid));
    formatter::print_line(&format!(
        "{:<16} {}",
        "Uptime:",
        formatter::format_uptime(info.uptime_secs)
    ));
    formatter::print_line(&format!("{:<16} {}", "Projects:", info.projects_managed));
}

fn print_worker_info(info: WorkerInfo) {
    formatter::print_line("CodeNav Worker");
    formatter::print_line(&format!("{:<16} {}", "Project ID:", info.project_id));
    formatter::print_line(&format!(
        "{:<16} {}",
        "Project Root:",
        info.project_root.display()
    ));
    formatter::print_line(&format!("{:<16} {}", "Version:", info.server_version));
    formatter::print_line(&format!("{:<16} {}", "PID:", info.pid));
    formatter::print_line(&format!(
        "{:<16} {}",
        "Uptime:",
        formatter::format_uptime(info.uptime_secs)
    ));
    if !info.config_summary.is_empty() {
        formatter::print_line(&format!("{:<16}", "Config:"));
        for (k, v) in info.config_summary {
            formatter::print_line(&format!("  - {:<14} {}", format!("{}:", k), v));
        }
    }
}

fn print_master_status(status: MasterStatus) {
    let mut rows = Vec::new();
    rows.push(vec![
        "ID".to_string(),
        "PATH".to_string(),
        "STATUS".to_string(),
        "INDEXED".to_string(),
        "UPTIME".to_string(),
    ]);

    for worker in status.workers {
        rows.push(vec![
            worker.project_id,
            worker.project_root.display().to_string(),
            serde_json::to_string(&worker.status).unwrap_or_default(),
            format!("{} files", worker.indexed_files_count),
            formatter::format_uptime(worker.uptime_secs),
        ]);
    }

    let mut widths = vec![0; rows[0].len()];
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
    }

    for (i, row) in rows.iter().enumerate() {
        let formatted = row
            .iter()
            .enumerate()
            .map(|(j, cell)| format!("{:<width$}", cell, width = widths[j]))
            .collect::<Vec<_>>()
            .join("  ");
        formatter::print_line(&formatted);
        if i == 0 {
            let separator = widths
                .iter()
                .map(|w| "-".repeat(*w))
                .collect::<Vec<_>>()
                .join("  ");
            formatter::print_line(&separator);
        }
    }
}




#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_exposes_project_subcommand() {
        let cmd = Cli::command();
        assert!(
            cmd.get_subcommands()
                .any(|sub| sub.get_name() == "project"),
            "`project` 子命令应该在顶级 CLI 中出现"
        );
    }

    #[test]
    fn project_subcommand_lists_all_actions() {
        let cmd = Cli::command();
        let project_cmd = cmd
            .find_subcommand("project")
            .expect("顶级命令应该包含 `project`");
        let mut actions: Vec<_> = project_cmd
            .get_subcommands()
            .map(|sub| sub.get_name())
            .collect();
        actions.sort();
        assert_eq!(
            actions,
            vec!["add", "list", "remove", "restart", "status"],
            "`project` 子命令应该暴露所有管理动作"
        );
    }
}
