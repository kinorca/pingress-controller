use pingora::tls::pkey::{PKey, Private};
use pingora::tls::x509::X509;
use pingress_config::{PingressConfiguration, Tls};
use std::collections::HashMap;
use std::fs::read;

pub(crate) struct TlsMap {
    tls: HashMap<String, (PKey<Private>, X509)>,
}

pub(crate) trait GetTls {
    fn get_tls(&self, host: &str) -> Option<(String, PKey<Private>, X509)>;
}

impl GetTls for TlsMap {
    fn get_tls(&self, host: &str) -> Option<(String, PKey<Private>, X509)> {
        let (pk, ct) = self.tls.get(host)?;
        Some((host.to_string(), pk.clone(), ct.clone()))
    }
}

impl From<PingressConfiguration> for TlsMap {
    fn from(value: PingressConfiguration) -> Self {
        Self {
            tls: value
                .rules
                .into_iter()
                .filter_map(|r| r.tls.map(|t| (r.host, t)))
                .map(|(host, tls)| (host, tls.into_key_cert()))
                .collect(),
        }
    }
}

trait IntoKeyCert {
    fn into_key_cert(self) -> (PKey<Private>, X509);
}

impl IntoKeyCert for Tls {
    fn into_key_cert(self) -> (PKey<Private>, X509) {
        let pk = read(self.key.as_str()).unwrap();
        let ct = read(self.cert.as_str()).unwrap();

        (
            PKey::private_key_from_pem(pk.as_slice()).unwrap(),
            X509::from_pem(ct.as_slice()).unwrap(),
        )
    }
}
