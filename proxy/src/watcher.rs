use crate::metrics::Metrics;
use crate::state::{ProxyState, SharedState};
use notify::{EventKind, RecursiveMode, Watcher, recommended_watcher};
use std::path::Path;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;
use tracing::{debug, error, info};

/// Events arriving within this period are handled as a single reload
const DEBOUNCE: Duration = Duration::from_millis(500);

/// Watch the directories and reload the state when anything in them changes.
///
/// ConfigMap and Secret volumes are updated by atomically swapping a `..data` symlink, which shows
/// up as create/remove events rather than modify events, so every event except access is a
/// trigger. The state is replaced only when the new configuration loads successfully.
pub(crate) fn run_reload(
    watch: &[String],
    config_file: &str,
    state: &SharedState,
    metrics: &Metrics,
) {
    let (tx, rx) = channel();
    let mut watcher = match recommended_watcher(tx) {
        Ok(w) => w,
        Err(e) => {
            error!(error = %e, "Cannot create file watcher, reload is disabled");
            return;
        }
    };
    for dir in watch {
        match watcher.watch(Path::new(dir), RecursiveMode::Recursive) {
            Ok(()) => info!(directory = %dir, "Watching for configuration changes"),
            Err(e) => error!(directory = %dir, error = %e, "Cannot watch directory"),
        }
    }

    loop {
        match rx.recv() {
            Ok(Ok(e)) if is_trigger(&e.kind) => {
                debug!(kind = ?e.kind, paths = ?e.paths, "File change detected");
            }
            Ok(Ok(_)) => continue,
            Ok(Err(e)) => {
                error!(error = %e, "File watcher error");
                continue;
            }
            Err(_) => {
                info!("File watcher stopped");
                return;
            }
        }

        // Wait until the burst of events settles down
        loop {
            match rx.recv_timeout(DEBOUNCE) {
                Ok(_) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }

        match ProxyState::load(config_file) {
            Ok(s) => {
                let (routes, tls_certificates) = (s.proxy_map.len(), s.tls.len());
                state.replace(s);
                metrics.config_loaded(routes, tls_certificates);
                info!(
                    config_file,
                    routes, tls_certificates, "Configuration reloaded"
                );
            }
            Err(e) => {
                metrics.config_load_failed();
                error!(
                    config_file,
                    error = %e,
                    "Cannot reload configuration, keeping the current one"
                );
            }
        }
    }
}

fn is_trigger(kind: &EventKind) -> bool {
    !matches!(kind, EventKind::Access(_))
}
