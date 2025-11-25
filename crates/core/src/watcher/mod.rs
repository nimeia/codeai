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
