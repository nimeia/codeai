use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Start watching `root` and invoke `on_change` when filesystem events arrive.
///
/// A simple debounce is applied to avoid excessive re-index triggers when a batch
/// of files change together.
pub fn start<F>(root: PathBuf, debounce: Duration, mut on_change: F) -> Result<()>
where
    F: FnMut() + Send + 'static,
{
    let (tx, rx) = mpsc::channel();

    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;

    watcher.watch(&root, RecursiveMode::Recursive)?;

    thread::spawn(move || {
        // Keep the watcher alive for the lifetime of the thread.
        let _watcher = watcher;
        let mut last_fired = Instant::now();
        while let Ok(event) = rx.recv() {
            match event {
                Ok(ev)
                    if matches!(
                        ev.kind,
                        EventKind::Any
                            | EventKind::Create(_)
                            | EventKind::Modify(_)
                            | EventKind::Remove(_)
                    ) =>
                {
                    if last_fired.elapsed() < debounce {
                        continue;
                    }
                    last_fired = Instant::now();
                    on_change();
                }
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(error = %err, "file watch event error");
                }
            }
        }
    });

    tracing::info!(root = %root.display(), "watcher initialized");
    Ok(())
}

fn build_snapshot(root: &Path) -> Result<HashMap<PathBuf, std::time::SystemTime>> {
    let mut snapshot = HashMap::new();
    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.into_path();
        let modified = fs::metadata(&path)
            .and_then(|m| m.modified())
            .with_context(|| format!("failed to read modified time for {}", path.display()))?;
        snapshot.insert(path, modified);
    }

    Ok(snapshot)
}

fn detect_changes(
    root: &Path,
    snapshot: &mut HashMap<PathBuf, std::time::SystemTime>,
) -> Result<bool> {
    let mut changed = false;
    let mut next_snapshot = HashMap::new();

    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.into_path();
        let modified = match fs::metadata(&path).and_then(|m| m.modified()) {
            Ok(time) => time,
            Err(err) => {
                tracing::warn!(error = %err, "failed to read modified time");
                continue;
            }
        };

        if let Some(previous) = snapshot.get(&path) {
            if modified > *previous {
                changed = true;
            }
        } else {
            changed = true;
        }

        next_snapshot.insert(path, modified);
    }

    // Detect deletions.
    if !changed {
        for missing_path in snapshot.keys() {
            if !next_snapshot.contains_key(missing_path) {
                changed = true;
                break;
            }
        }
    }

    *snapshot = next_snapshot;
    Ok(changed)
}
