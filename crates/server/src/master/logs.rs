use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::Utc;
use code_nav_protocol::{LogEvent, LogLevel, LogTarget, LogsRequest, LogsResponse, Response};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

const DEFAULT_HISTORY_CAP: usize = 10_000;

static GLOBAL_LOGS: OnceLock<Arc<LogsService>> = OnceLock::new();

pub fn init_global(capacity: usize) -> Arc<LogsService> {
    GLOBAL_LOGS
        .get_or_init(|| Arc::new(LogsService::new(capacity)))
        .clone()
}

pub fn global() -> Option<Arc<LogsService>> {
    GLOBAL_LOGS.get().cloned()
}

#[derive(Debug)]
pub struct LogsService {
    buffer: Mutex<VecDeque<LogEvent>>,
    capacity: usize,
}

impl LogsService {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity: capacity.max(1),
        }
    }

    pub fn handle_request(&self, request: LogsRequest) -> Response {
        match request.target {
            LogTarget::Master => {
                if request.follow {
                    return Response::Error(code_nav_protocol::ErrorBody {
                        code: code_nav_protocol::ErrorCode::Unsupported,
                        message: "follow 模式暂未实现".to_string(),
                    });
                }
                let events = self.collect(request.since, request.limit, request.level);
                Response::Logs(LogsResponse { events })
            }
            LogTarget::Worker(_) => Response::Error(code_nav_protocol::ErrorBody {
                code: code_nav_protocol::ErrorCode::Unsupported,
                message: "worker 日志暂未实现".to_string(),
            }),
        }
    }

    pub fn record(&self, mut event: LogEvent) {
        let mut guard = self
            .buffer
            .lock()
            .expect("logs buffer poisoned while recording event");

        if guard.len() == self.capacity {
            guard.pop_front();
        }

        event.ts = Utc::now().timestamp();
        guard.push_back(event);
    }

    fn collect(
        &self,
        since: Option<i64>,
        limit: Option<u32>,
        level: Option<LogLevel>,
    ) -> Vec<LogEvent> {
        let guard = self
            .buffer
            .lock()
            .expect("logs buffer poisoned while collecting history");

        let mut filtered: Vec<LogEvent> = guard
            .iter()
            .filter(|ev| {
                if let Some(since_ts) = since {
                    if ev.ts < since_ts {
                        return false;
                    }
                }
                if let Some(level_filter) = level.as_ref() {
                    if level_rank(&ev.level) < level_rank(level_filter) {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();

        if let Some(max) = limit {
            let max = max as usize;
            if filtered.len() > max {
                filtered.drain(0..filtered.len() - max);
            }
        }

        filtered
    }
}

#[derive(Clone)]
pub struct CaptureLayer {
    service: Arc<LogsService>,
    source: String,
}

impl CaptureLayer {
    pub fn new(service: Arc<LogsService>, source: impl Into<String>) -> Self {
        Self {
            service,
            source: source.into(),
        }
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);

        let metadata = event.metadata();
        let target = metadata.target();
        let message = visitor
            .message
            .unwrap_or_else(|| format!("{}", metadata.level()));
        let log_event = LogEvent {
            ts: Utc::now().timestamp(),
            level: match *metadata.level() {
                tracing::Level::TRACE => LogLevel::Trace,
                tracing::Level::DEBUG => LogLevel::Debug,
                tracing::Level::INFO => LogLevel::Info,
                tracing::Level::WARN => LogLevel::Warn,
                tracing::Level::ERROR => LogLevel::Error,
            },
            source: self.source.clone(),
            target: Some(target.to_string()),
            message,
            fields: visitor.fields,
        };

        self.service.record(log_event);
    }
}

#[derive(Default)]
struct EventVisitor {
    message: Option<String>,
    fields: std::collections::BTreeMap<String, String>,
}

impl tracing::field::Visit for EventVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let rendered = format!("{:?}", value);
        if field.name() == "message" {
            self.message = Some(rendered);
        } else {
            self.fields.insert(field.name().to_string(), rendered);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }
    }
}

pub fn history_capacity_or_default(value: Option<usize>) -> usize {
    value.unwrap_or(DEFAULT_HISTORY_CAP)
}

fn level_rank(level: &LogLevel) -> u8 {
    match level {
        LogLevel::Trace => 0,
        LogLevel::Debug => 1,
        LogLevel::Info => 2,
        LogLevel::Warn => 3,
        LogLevel::Error => 4,
    }
}
