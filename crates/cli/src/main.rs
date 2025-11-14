use anyhow::Result;
use clap::{Parser, Subcommand};
use code_nav_protocol::{ListKind, Request};

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
    let request = match cli.command {
        Commands::Search { query, top_k } => {
            Request::Search(code_nav_protocol::SearchRequest { query, top_k })
        }
        Commands::List { kind } => {
            let list_kind = match kind {
                Kind::Classes => ListKind::Classes,
                Kind::Methods => ListKind::Methods,
                Kind::Files => ListKind::Files,
            };
            Request::List(code_nav_protocol::ListRequest {
                kind: list_kind,
                filter: None,
                limit: None,
            })
        }
    };

    // TODO: send request to server; placeholder prints JSON.
    println!("{}", serde_json::to_string(&request)?);
    Ok(())
}
