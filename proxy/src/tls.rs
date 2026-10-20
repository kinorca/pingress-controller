use crate::proxy_map::wildcard_suffix;
use log::error;
use pingora::tls::pkey::{PKey, Private};
use pingora::tls::x509::X509;
use pingress_config::{PingressConfiguration, Tls};
use std::collections::HashMap;
use std::fs::read;

pub(crate) struct TlsMap {
    tls: HashMap<String, (PKey<Private>, X509)>,
}

impl TlsMap {
    /// Find a key pair for the host, falling back to a wildcard certificate (`*.example.com`)
    pub(crate) fn get_tls(&self, host: &str) -> Option<&(PKey<Private>, X509)> {
        let host = host.to_ascii_lowercase();
        self.tls.get(&host).or_else(|| {
            let suffix = wildcard_suffix(&host)?;
            self.tls.get(&format!("*.{suffix}"))
        })
    }

    pub(crate) fn contains(&self, host: &str) -> bool {
        self.get_tls(host).is_some()
    }
}

impl From<PingressConfiguration> for TlsMap {
    /// Hosts whose key pair cannot be loaded are logged and skipped, so that one broken
    /// certificate does not take the whole proxy down.
    fn from(value: PingressConfiguration) -> Self {
        Self {
            tls: value
                .rules
                .into_iter()
                .filter_map(|r| Some((r.host?.to_ascii_lowercase(), r.tls?)))
                .filter_map(|(host, tls)| match tls.load_key_cert() {
                    Ok(kc) => Some((host, kc)),
                    Err(e) => {
                        error!("Cannot load TLS key pair for '{host}': {e}");
                        None
                    }
                })
                .collect(),
        }
    }
}

trait LoadKeyCert {
    fn load_key_cert(&self) -> Result<(PKey<Private>, X509), String>;
}

impl LoadKeyCert for Tls {
    fn load_key_cert(&self) -> Result<(PKey<Private>, X509), String> {
        let pk = read(&self.key).map_err(|e| format!("{}: {e}", self.key))?;
        let ct = read(&self.cert).map_err(|e| format!("{}: {e}", self.cert))?;

        Ok((
            PKey::private_key_from_pem(&pk).map_err(|e| format!("{}: {e}", self.key))?,
            X509::from_pem(&ct).map_err(|e| format!("{}: {e}", self.cert))?,
        ))
    }
}
