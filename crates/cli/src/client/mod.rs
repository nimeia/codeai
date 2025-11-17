use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn get_socket_path() -> Result<PathBuf> {
    // This logic is adapted from `crates/cli/src/commands/project.rs`
    let home = dirs::home_dir().context("failed to determine home directory")?;
    let runtime_dir = home.join(".code-nav");
    Ok(runtime_dir.join("master.sock"))
}

pub fn send_request(payload: &str) -> Result<Option<String>> {
    let socket_path = get_socket_path()?;
    match UnixStream::connect(&socket_path) {
        Ok(mut stream) => {
            tracing::debug!(payload, "sending request");
            stream
                .write_all(payload.as_bytes())
                .context("failed to write to socket")?;

            let mut response = String::new();
            stream
                .read_to_string(&mut response)
                .context("failed to read from socket")?;
            tracing::debug!(response, "received response");
            Ok(Some(response))
        }
        Err(e) => {
            tracing::warn!(
                "could not connect to daemon at {}: {}. Is the daemon running?",
                socket_path.display(),
                e
            );
            println!("Note: Could not connect to code-nav daemon. Operations will be local-only.");
            Ok(None)
        }
    }
}
