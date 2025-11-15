mod client;
mod commands;
mod formatter;

use anyhow::Result;
use clap::{Parser, Subcommand};
use code_nav_protocol::{ListKind, Request};
use commands::{
    logs::{self, LogsArgs},
    project::{self, ProjectCommand},
};

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
