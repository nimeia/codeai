mod client;
mod commands;
mod formatter;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use code_nav_protocol::{
    InfoRequest, ListKind, ProjectRef, Request, StatusRequest, StatusResponse,
};
use commands::{
    logs::{self, LogsArgs},
    project::{self, ProjectCommand},
};
use std::path::Path;

#[derive(Parser)]
#[command(name = "code-nav", version, about = "code navigation cli")]
struct Cli {
    /// Address of the code-nav daemon (e.g., http://localhost:6688, unix:///tmp/code-nav.sock)
    #[arg(
        long = "connect",
        global = true,
        default_value = "http://localhost:6688"
    )]
    connect: String,
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

pub struct CliContext {
    client: Box<dyn client::RpcClient>,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::try_init().ok();
    let cli = Cli::parse();

    let rpc_client = client::new_rpc_client(&cli.connect)?;
    let ctx = CliContext { client: rpc_client };

    match cli.command {
        Commands::Search { query, top_k } => {
            handle_search(&ctx, query, top_k)?;
        }
        Commands::List { kind } => {
            handle_list(&ctx, kind)?;
        }
        Commands::Project { action } => {
            project::run(&ctx, action)?;
        }
        Commands::Logs(args) => {
            logs::run(&ctx, args)?;
        }
        Commands::Info(args) => {
            handle_info(&ctx, args)?;
        }
        Commands::Status(args) => {
            handle_status(&ctx, args)?;
        }
    }
    Ok(())
}

fn handle_search(ctx: &CliContext, query: String, top_k: u32) -> Result<()> {
    let request = Request::Search(code_nav_protocol::SearchRequest { query, top_k });
    let response = ctx.client.send(&request)?;
    // TODO: process search response
    formatter::print_line(&serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn handle_list(ctx: &CliContext, kind: Kind) -> Result<()> {
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
    let response = ctx.client.send(&request)?;
    // TODO: process list response
    formatter::print_line(&serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn handle_info(ctx: &CliContext, args: InfoArgs) -> Result<()> {
    let target = args.project.map(|p| {
        if Path::new(&p).exists() {
            ProjectRef::Path(p)
        } else {
            ProjectRef::Id(p)
        }
    });

    let request = Request::Info(InfoRequest { target });
    let response = ctx.client.send(&request)?;

    if let code_nav_protocol::Response::Info(info_response) = response {
        if args.json {
            formatter::print_line(&serde_json::to_string_pretty(&info_response)?);
        } else {
            formatter::print_info_response(info_response);
        }
    } else {
        formatter::print_line("Error: received unexpected response type from server");
    }
    Ok(())
}

fn handle_status(ctx: &CliContext, args: StatusArgs) -> Result<()> {
    let request = Request::Status(StatusRequest { target: None });
    let response = ctx.client.send(&request)?;

    if let code_nav_protocol::Response::Status(StatusResponse::Master(master_status)) = response {
        if args.json {
            formatter::print_line(&serde_json::to_string_pretty(&master_status)?);
        } else {
            formatter::print_master_status(master_status);
        }
    } else {
        formatter::print_line("Error: received unexpected response type from server");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_exposes_project_subcommand() {
        let cmd = Cli::command();
        assert!(
            cmd.get_subcommands().any(|sub| sub.get_name() == "project"),
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
