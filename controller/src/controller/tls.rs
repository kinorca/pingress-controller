use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;

/// Key of the certificate in a `kubernetes.io/tls` Secret
pub(super) const TLS_CERT_KEY: &str = "tls.crt";
/// Key of the private key in a `kubernetes.io/tls` Secret
pub(super) const TLS_PRIVATE_KEY_KEY: &str = "tls.key";

/// Base name of the TLS files of the host. Secret keys cannot contain `*`, so it is replaced by
/// `_`, which never appears in a valid Ingress host.
fn tls_file_stem(host: &str) -> String {
    host.replace('*', "_")
}

/// File name in the proxy (and key in the TLS secret) of the certificate of the host
pub(super) fn tls_cert_file(host: &str) -> String {
    format!("{}.crt", tls_file_stem(host))
}

/// File name in the proxy (and key in the TLS secret) of the private key of the host
pub(super) fn tls_key_file(host: &str) -> String {
    format!("{}.key", tls_file_stem(host))
}

pub(super) trait ExtractTls {
    fn extract_tls(&self) -> Option<(ByteString, ByteString)>;
}

impl ExtractTls for Secret {
    fn extract_tls(&self) -> Option<(ByteString, ByteString)> {
        let cert = self.data.as_ref()?.get(TLS_CERT_KEY)?;
        let key = self.data.as_ref()?.get(TLS_PRIVATE_KEY_KEY)?;

        Some((cert.clone(), key.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::{ExtractTls, tls_cert_file, tls_key_file};
    use k8s_openapi::ByteString;
    use k8s_openapi::api::core::v1::Secret;
    use std::collections::BTreeMap;

    fn secret(data: &[(&str, &[u8])]) -> Secret {
        Secret {
            data: Some(
                data.iter()
                    .map(|(k, v)| (k.to_string(), ByteString(v.to_vec())))
                    .collect::<BTreeMap<_, _>>(),
            ),
            ..Secret::default()
        }
    }

    #[test]
    fn extract_tls() {
        let s = secret(&[("tls.crt", b"cert"), ("tls.key", b"key")]);

        let (cert, key) = s.extract_tls().expect("tls data exists");
        assert_eq!(cert.0, b"cert");
        assert_eq!(key.0, b"key");
    }

    #[test]
    fn extract_tls_missing_key() {
        let s = secret(&[("tls.crt", b"cert")]);

        assert!(s.extract_tls().is_none());
    }

    #[test]
    fn extract_tls_without_data() {
        assert!(Secret::default().extract_tls().is_none());
    }

    #[test]
    fn tls_file_names() {
        assert_eq!(tls_cert_file("example.com"), "example.com.crt");
        assert_eq!(tls_key_file("*.example.com"), "_.example.com.key");
    }
}
