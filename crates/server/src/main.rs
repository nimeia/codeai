use anyhow::Result;
use code-nav-core::{indexer, search};
use code-nav-protocol::{Envelope, Request, Response};

fn main() -> Result<()> {
    tracing_subscriber::fmt::try_init().ok();
    tracing::info!("code-nav server booting");

    // Placeholder: respond to a mock search request to show wiring.
    let req = Envelope {
        id: "0".into(),
        data: Request::Search(code_nav_protocol::SearchRequest {
            query: "example".into(),
            top_k: 5,
        }),
    };
    let resp = handle_request(req.data);
    println!("{}", serde_json::to_string(&resp)?);
    Ok(())
}

fn handle_request(req: Request) -> Response {
    match req {
        Request::Search(payload) => {
            let hits = search::semantic(&payload.query, payload.top_k as usize)
                .into_iter()
                .map(|hit| code_nav_protocol::SearchHit {
                    file: hit.file,
                    line: hit.line,
                    score: hit.score,
                    snippet: String::from("placeholder"),
                })
                .collect();
            Response::Search(code_nav_protocol::SearchResponse { hits })
        }
        Request::Index(job) => {
            let _ = indexer::run(code_nav_core::indexer::IndexJob {
                full: matches!(job.mode, code_nav_protocol::IndexMode::Full),
            });
            Response::Index(code_nav_protocol::IndexResponse { started: true })
        }
        Request::Info => Response::Info(code_nav_protocol::InfoResponse {
            protocol_version: "0.1.0".into(),
        }),
        Request::Status => Response::Status(Default::default()),
        Request::Goto(_) => Response::Goto(Default::default()),
        Request::List(_) => Response::List(Default::default()),
    }
}
