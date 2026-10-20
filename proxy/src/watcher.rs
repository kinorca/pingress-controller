use crate::state::{ProxyState, SharedState};
use log::{error, info};
use notify::{EventKind, RecursiveMode, Watcher, recommended_watcher};
use std::path::Path;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

/// Events arriving within this period are handled as a single reload
const DEBOUNCE: Duration = Duration::from_millis(500);

/// Watch the directories and reload the state when anything in them changes.
///
/// ConfigMap and Secret volumes are updated by atomically swapping a `..data` symlink, which shows
/// up as create/remove events rather than modify events, so every event except access is a
/// trigger. The state is replaced only when the new configuration loads successfully.
pub(crate) fn run_reload(watch: &[String], config_file: &str, state: &SharedState) {
    let (tx, rx) = channel();
    let mut watcher = match recommended_watcher(tx) {
        Ok(w) => w,
        Err(e) => {
            error!("Cannot create file watcher, reload is disabled: {e}");
            return;
        }
    };
    for dir in watch {
        if let Err(e) = watcher.watch(Path::new(dir), RecursiveMode::Recursive) {
            error!("Cannot watch '{dir}': {e}");
        }
    }

    loop {
        match rx.recv() {
            Ok(Ok(e)) if is_trigger(&e.kind) => {}
            Ok(Ok(_)) => continue,
            Ok(Err(e)) => {
                error!("Event error: {e}");
                continue;
            }
            Err(_) => return,
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
                state.replace(s);
                info!("Configuration reloaded");
            }
            Err(e) => error!("Cannot reload configuration, keeping the current one: {e}"),
        }
    }
}

fn is_trigger(kind: &EventKind) -> bool {
    !matches!(kind, EventKind::Access(_))
}
