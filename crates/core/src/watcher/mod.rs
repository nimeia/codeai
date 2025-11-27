use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use walkdir::WalkDir;

/// Start watching `root` and invoke `on_change` when filesystem events arrive.
///
/// A simple debounce is applied to avoid excessive re-index triggers when a batch
/// of files change together.
pub fn start<F>(root: PathBuf, debounce: Duration, mut on_change: F) -> Result<()>
where
    F: FnMut() + Send + 'static,
{
    let mut snapshot = build_snapshot(&root)?;
    let watch_root = root.clone();

    thread::spawn(move || {
        let mut last_fired = Instant::now();

        loop {
            match detect_changes(&watch_root, &mut snapshot) {
                Ok(true) if last_fired.elapsed() >= debounce => {
                    last_fired = Instant::now();
                    on_change();
                }
                Ok(true) | Ok(false) => {}
                Err(err) => {
                    tracing::warn!(error = %err, "file watch poll error");
                }
            }

            thread::sleep(debounce);
        }
    });

    tracing::info!(root = %root.display(), "watcher initialized");
    Ok(())
}

fn build_snapshot(root: &Path) -> Result<HashMap<PathBuf, std::time::SystemTime>> {
    let mut snapshot = HashMap::new();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
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

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
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
