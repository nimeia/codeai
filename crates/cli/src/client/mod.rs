use anyhow::{bail, Context, Result};
use code_nav_protocol::{Request, Response};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use url::Url;

pub trait RpcClient: Send + Sync {
    fn send(&self, request: &Request) -> Result<Response>;
}

pub struct UnixSocketClient {
    path: PathBuf,
}

impl UnixSocketClient {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl RpcClient for UnixSocketClient {
    fn send(&self, request: &Request) -> Result<Response> {
        let payload = serde_json::to_string(request)?;
        match UnixStream::connect(&self.path) {
            Ok(mut stream) => {
                tracing::debug!(payload, "sending request via UDS");
                stream
                    .write_all(payload.as_bytes())
                    .context("failed to write to UDS")?;

                let mut response_payload = String::new();
                stream
                    .read_to_string(&mut response_payload)
                    .context("failed to read from UDS")?;
                tracing::debug!(response_payload, "received response via UDS");
                let response: Response = serde_json::from_str(&response_payload)
                    .context("failed to deserialize response from UDS")?;
                Ok(response)
            }
            Err(e) => {
                bail!(
                    "could not connect to daemon at {}: {}. Is the daemon running?",
                    self.path.display(),
                    e
                );
            }
        }
    }
}

pub struct HttpClient {
    url: Url,
    client: reqwest::blocking::Client,
}

impl HttpClient {
    pub fn new(url: Url) -> Self {
        Self {
            url,
            client: reqwest::blocking::Client::new(),
        }
    }
}

impl RpcClient for HttpClient {
    fn send(&self, request: &Request) -> Result<Response> {
        tracing::debug!(?request, "sending request via HTTP");
        let response = self
            .client
            .post(self.url.as_str())
            .json(request)
            .send()
            .context("failed to send HTTP request")?
            .error_for_status()
            .context("HTTP request failed with non-success status")?;

        let response_payload = response
            .text()
            .context("failed to read HTTP response body")?;
        tracing::debug!(response_payload, "received response via HTTP");
        let response: Response = serde_json::from_str(&response_payload)
            .context("failed to deserialize response from HTTP")?;
        Ok(response)
    }
}

pub fn new_rpc_client(address: &str) -> Result<Box<dyn RpcClient>> {
    let parsed_url = Url::parse(address).context("invalid connection address format")?;

    match parsed_url.scheme() {
        "unix" => {
            let path = PathBuf::from(parsed_url.path());
            if !path.is_absolute() {
                bail!("UDS path must be absolute: {}", parsed_url.path());
            }
            Ok(Box::new(UnixSocketClient::new(path)))
        }
        "http" | "https" => {
            let url = normalize_http_rpc_url(parsed_url);
            Ok(Box::new(HttpClient::new(url)))
        }
        _ => bail!("unsupported connection scheme: {}", parsed_url.scheme()),
    }
}

fn normalize_http_rpc_url(mut url: Url) -> Url {
    if url.path() == "/" || url.path().is_empty() {
        url.set_path("/rpc");
    }

    url
}
