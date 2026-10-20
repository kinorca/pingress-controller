use crate::proxy_map::ProxyMap;
use crate::tls::TlsMap;
use pingress_config::PingressConfiguration;
use std::fs::File;
use std::sync::{Arc, RwLock};

/// Routing table and TLS key pairs built from one configuration file
pub(crate) struct ProxyState {
    pub(crate) proxy_map: ProxyMap,
    pub(crate) tls: TlsMap,
}

impl ProxyState {
    pub(crate) fn load(config_file: &str) -> Result<Self, String> {
        let file = File::open(config_file).map_err(|e| format!("{config_file}: {e}"))?;
        let config: PingressConfiguration =
            serde_json::from_reader(file).map_err(|e| format!("{config_file}: {e}"))?;

        Ok(Self {
            proxy_map: ProxyMap::from(config.clone()),
            tls: TlsMap::from(config),
        })
    }
}

/// The current [`ProxyState`], replaced as a whole on reload
#[derive(Clone)]
pub(crate) struct SharedState(Arc<RwLock<Arc<ProxyState>>>);

impl SharedState {
    pub(crate) fn new(state: ProxyState) -> Self {
        Self(Arc::new(RwLock::new(Arc::new(state))))
    }

    pub(crate) fn get(&self) -> Arc<ProxyState> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn replace(&self, state: ProxyState) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(state);
    }
}
