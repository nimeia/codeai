pub fn print_line(line: &str) {
    println!("{line}");
}

pub fn format_uptime(total_seconds: u64) -> String {
    let days = total_seconds / (24 * 3600);
    let hours = (total_seconds % (24 * 3600)) / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{}d", days));
    }
    if hours > 0 {
        parts.push(format!("{}h", hours));
    }
    if minutes > 0 {
        parts.push(format!("{}m", minutes));
    }
    if seconds > 0 || parts.is_empty() {
        parts.push(format!("{}s", seconds));
    }

    parts.join(" ")
}

use code_nav_protocol::{InfoResponse, MasterInfo, MasterStatus, WorkerInfo, WorkerStatus};
use serde_json;

pub fn print_info_response(response: InfoResponse) {
    match response {
        InfoResponse::Master(info) => print_master_info(info),
        InfoResponse::Worker(info) => print_worker_info(info),
    }
}

pub fn print_master_info(info: MasterInfo) {
    print_line("CodeNav Master");
    print_line(&format!("{:<16} {}", "Version:", info.server_version));
    print_line(&format!("{:<16} {}", "Protocol:", info.protocol_version));
    print_line(&format!("{:<16} {}", "PID:", info.pid));
    print_line(&format!(
        "{:<16} {}",
        "Uptime:",
        format_uptime(info.uptime_secs)
    ));
    print_line(&format!("{:<16} {}", "Projects:", info.projects_managed));
}

pub fn print_worker_info(info: WorkerInfo) {
    print_line("CodeNav Worker");
    print_line(&format!("{:<16} {}", "Project ID:", info.project_id));
    print_line(&format!(
        "{:<16} {}",
        "Project Root:",
        info.project_root.display()
    ));
    print_line(&format!("{:<16} {}", "Version:", info.server_version));
    print_line(&format!("{:<16} {}", "PID:", info.pid));
    print_line(&format!(
        "{:<16} {}",
        "Uptime:",
        format_uptime(info.uptime_secs)
    ));
    if !info.config_summary.is_empty() {
        print_line(&format!("{:<16}", "Config:"));
        for (k, v) in info.config_summary {
            print_line(&format!("  - {:<14} {}", format!("{}:", k), v));
        }
    }
}

pub fn print_master_status(status: MasterStatus) {
    print_line("CodeNav Master Status");
    print_line(&format!("{:<12} {}", "PID:", status.pid));
    print_line(&format!(
        "{:<12} {}",
        "Uptime:",
        format_uptime(status.uptime_secs)
    ));
    print_line(&format!("{:<12} {}", "Workers:", status.workers.len()));
    if status.workers.is_empty() {
        return;
    }

    let mut rows = Vec::new();
    rows.push(vec![
        "ID".to_string(),
        "PID".to_string(),
        "PATH".to_string(),
        "STATUS".to_string(),
        "INDEXED".to_string(),
        "UPTIME".to_string(),
    ]);

    for worker in status.workers {
        rows.push(vec![
            worker.project_id,
            worker.pid.to_string(),
            worker.project_root.display().to_string(),
            serde_json::to_string(&worker.status).unwrap_or_default(),
            format!("{} files", worker.indexed_files_count),
            format_uptime(worker.uptime_secs),
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
        print_line(&formatted);
        if i == 0 {
            let separator = widths
                .iter()
                .map(|w| "-".repeat(*w))
                .collect::<Vec<_>>()
                .join("  ");
            print_line(&separator);
        }
    }
}

pub fn print_worker_status(status: WorkerStatus) {
    let summary = status.summary;
    print_line(&format!(
        "Project: {} ({})",
        summary.project_id,
        summary.project_root.display()
    ));

    let status_string = match status.indexer_state.state {
        code_nav_protocol::WorkerState::Indexing => {
            if let Some(percent) = status.indexer_state.progress_percent {
                format!("Indexing ({:.1}%)", percent)
            } else {
                "Indexing".to_string()
            }
        }
        _ => serde_json::to_string(&summary.status).unwrap_or_default(),
    };
    print_line(&format!("{:<12} {}", "Status:", status_string));

    if let Some(file) = status.indexer_state.current_file {
        if !file.is_empty() {
            print_line(&format!("{:<12} {}", "File:", file));
        }
    }

    print_line(&format!(
        "{:<12} {}",
        "Uptime:",
        format_uptime(summary.uptime_secs)
    ));

    let watcher_status = if status.is_watching {
        "Active"
    } else {
        "Inactive"
    };
    print_line(&format!("{:<12} {}", "Watcher:", watcher_status));
    print_line(&format!(
        "{:<12} {} tasks pending",
        "Queue:", status.task_queue_size
    ));
}
