use std::{fmt, path::Path};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use clap::{Args, ValueEnum};
use code_nav_protocol::{LogLevel, LogOutputFormat, LogTarget, LogsRequest, ProjectRef, Request};


use crate::formatter;
use crate::CliContext; // Add CliContext

#[derive(Debug, Args)]
pub struct LogsArgs {
    /// Target log stream. Defaults to master when omitted.
    #[arg(long = "target", value_enum, default_value_t = LogTargetArg::Master)]
    pub target: LogTargetArg,
    /// Project path or identifier when requesting worker logs.
    #[arg(long = "project", value_name = "PATH|ID")]
    pub project: Option<String>,
    /// Only return events newer than the provided duration or RFC3339 timestamp.
    #[arg(long = "since", value_name = "DURATION|RFC3339")]
    pub since: Option<String>,
    /// Maximum number of historical events to fetch (default 500, max 10k).
    #[arg(long = "limit", value_name = "COUNT", default_value_t = 500)]
    pub limit: u32,
    /// Continue streaming logs after the initial history snapshot.
    #[arg(long = "follow", short = 'f')]
    pub follow: bool,
    /// Polling interval in milliseconds for follow mode.
    #[arg(long = "follow-interval", value_name = "MILLIS", default_value_t = 250)]
    pub follow_interval_ms: u64,
    /// Optional minimum log level filter.
    #[arg(long = "level", value_enum)]
    pub level: Option<LogLevelArg>,
    /// Emit JSON log events instead of formatted text.
    #[arg(long = "json")]
    pub json: bool,
    /// Control ANSI color usage when rendering text output.
    #[arg(long = "color", value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum LogTargetArg {
    Master,
    Worker,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum LogLevelArg {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<LogLevelArg> for LogLevel {
    fn from(value: LogLevelArg) -> Self {
        match value {
            LogLevelArg::Trace => LogLevel::Trace,
            LogLevelArg::Debug => LogLevel::Debug,
            LogLevelArg::Info => LogLevel::Info,
            LogLevelArg::Warn => LogLevel::Warn,
            LogLevelArg::Error => LogLevel::Error,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

impl fmt::Display for ColorChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorChoice::Auto => write!(f, "auto"),
            ColorChoice::Always => write!(f, "always"),
            ColorChoice::Never => write!(f, "never"),
        }
    }
}

// LogsPreview is removed

pub fn run(ctx: &CliContext, args: LogsArgs) -> Result<()> {
    if args.limit == 0 {
        bail!("--limit 必须为正数");
    }
    let limit = args.limit.min(10_000);
    let target = build_target(&args)?;
    let since = match args.since.as_deref() {
        Some(value) => Some(parse_since(value)?),
        None => None,
    };
    let request_payload = LogsRequest {
        target,
        since,
        limit: Some(limit),
        follow: args.follow,
        follow_interval_ms: Some(args.follow_interval_ms),
        level: args.level.map(Into::into),
        format: if args.json {
            LogOutputFormat::Json
        } else {
            LogOutputFormat::Text
        },
    };

    let response = ctx.client.send(&Request::Logs(request_payload))?;

    if let code_nav_protocol::Response::Logs(logs_response) = response {
        if args.json {
            formatter::print_line(&serde_json::to_string_pretty(&logs_response)?);
        } else {
            // Placeholder: just print messages for now.
            for event in logs_response.events {
                formatter::print_line(&format!(
                    "[{:?}] {}: {}",
                    event.level, event.source, event.message
                ));
            }
        }
    } else {
        bail!("Error: received unexpected response type from server");
    }

    Ok(())
}

fn build_target(args: &LogsArgs) -> Result<LogTarget> {
    match args.target {
        LogTargetArg::Master => Ok(LogTarget::Master),
        LogTargetArg::Worker => {
            let project = args
                .project
                .as_ref()
                .context("`--target worker` 需要额外提供 --project")?;
            Ok(LogTarget::Worker(parse_project_ref(project)))
        }
    }
}

fn parse_project_ref(value: &str) -> ProjectRef {
    let path = Path::new(value);
    if path.exists() || value.contains('/') || value.contains('\\') {
        ProjectRef::Path(value.into())
    } else {
        ProjectRef::Id(value.into())
    }
}

fn parse_since(value: &str) -> Result<i64> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Ok(dt.timestamp());
    }

    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.len() < 2 {
        bail!("无法解析 --since: {{value}}");
    }
    let (number, unit) = trimmed.split_at(trimmed.len() - 1);
    let quantity: i64 = number
        .parse()
        .with_context(|| format!("无法解析持续时间 {{number}}"))?;
    let seconds = match unit {
        "s" => quantity,
        "m" => quantity * 60,
        "h" => quantity * 60 * 60,
        "d" => quantity * 60 * 60 * 24,
        _ => bail!("未知时间单位: {{unit}}"),
    };
    Ok(Utc::now().timestamp() - seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_since_understands_rfc3339() {
        let ts = parse_since("2024-01-02T03:04:05Z").unwrap();
        let expected = chrono::DateTime::parse_from_rfc3339("2024-01-02T03:04:05Z")
            .unwrap()
            .timestamp();
        assert_eq!(ts, expected);
    }

    #[test]
    fn parse_since_supports_relative_units() {
        let now = Utc::now().timestamp();
        let ts = parse_since("1h").unwrap();
        assert!(now - ts - 3600 < 5);
    }
}