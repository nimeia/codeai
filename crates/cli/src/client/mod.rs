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

pub fn send_request(payload: &str) -> Result<()> {
    let socket_path = get_socket_path()?;
    match UnixStream::connect(&socket_path) {
        Ok(mut stream) => {
            tracing::debug!(payload, "sending request");
            stream
                .write_all(payload.as_bytes())
                .context("failed to write to socket")?;

            // For now, we don't expect a response, or the response is handled elsewhere.
            // In a real implementation, we would read the response here.
            let mut response = String::new();
            stream
                .read_to_string(&mut response)
                .context("failed to read from socket")?;
            tracing::debug!(response, "received response");
            // TODO: Deserialize and handle the response properly.
            println!("Response from server (raw): {}", response);

            Ok(())
        }
        Err(e) => {
            tracing::warn!(
                "could not connect to daemon at {}: {}. Is the daemon running?",
                socket_path.display(),
                e
            );
            // This is not a critical error for now, as the server is not implemented.
            // The CLI commands still perform local actions.
            // In the future, this might become a hard error for most commands.
            println!("Note: Could not connect to code-nav daemon. Operations will be local-only.");
            Ok(())
        }
    }
}