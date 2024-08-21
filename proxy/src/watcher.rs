use crate::tls::TlsMap;
use log::error;
use nix::sys::signal::Signal;
use notify::{recommended_watcher, RecursiveMode, Watcher};
use pingress_config::PingressConfiguration;
use std::fs::File;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::mpsc::channel;
use std::sync::{Arc, RwLock};

pub(crate) fn run_reload(watch: &str, config: &str, tls: &Arc<RwLock<TlsMap>>) {
    let (tx, rx) = channel();
    let mut watcher = recommended_watcher(tx).unwrap();
    let path = PathBuf::from_str(watch).unwrap();
    watcher
        .watch(path.as_path(), RecursiveMode::Recursive)
        .unwrap();

    for event in rx {
        match event {
            Ok(e) => {
                if e.kind.is_modify() {
                    reload_tls_map(&tls, config);

                    let my_pid = nix::unistd::Pid::this();
                    if let Err(e) = nix::sys::signal::kill(my_pid, Signal::SIGTERM) {
                        error!("Cannot send signal {} to {my_pid}: {e}", Signal::SIGTERM);
                    }
                }
            }
            Err(e) => {
                error!("Event error: {e}");
            }
        }
    }
}

fn reload_tls_map(tls: &Arc<RwLock<TlsMap>>, config: &str) {
    let config: PingressConfiguration = {
        let file = File::open(config).unwrap();
        serde_json::from_reader(file).unwrap()
    };

    match tls.write() {
        Ok(mut t) => {
            *t = config.into();
        }
        Err(e) => {
            error!("Error: Cannot lock tls map: {e}");
        }
    }
}
