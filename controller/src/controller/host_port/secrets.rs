use crate::controller::host_port::ingresses::GetFromIngresses;
use crate::controller::host_port::{
    Context, FIELD_MANAGER, TLS_SECRET_NAME, manifest_labels, tls_cert_file, tls_key_file,
};
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::api::networking::v1::Ingress;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Api;
use kube::api::{DeleteParams, Patch, PatchParams};
use std::collections::{BTreeMap, HashSet};
use tracing::{info, warn};

/// Collect key pairs referenced by the Ingresses into the TLS secret, and return the hosts whose
/// key pair is available. Missing or malformed secrets are skipped so that they do not block the
/// other Ingresses.
pub(super) async fn apply_tls_secrets(
    ctx: &Context,
    ingresses: &[Ingress],
) -> Result<HashSet<String>, kube::Error> {
    let mut data = BTreeMap::new();
    let mut hosts = HashSet::new();
    for s in ingresses.tls_secrets() {
        let api: Api<Secret> = Api::namespaced(ctx.client.clone(), s.namespace.as_str());
        let Some(secret) = api.get_opt(s.secret.as_str()).await? else {
            warn!(
                secret_namespace = s.namespace,
                secret_name = s.secret,
                host = s.host,
                "TLS secret is not found"
            );
            continue;
        };
        let Some((cert, key)) = secret.extract_tls() else {
            warn!(
                secret_namespace = s.namespace,
                secret_name = s.secret,
                host = s.host,
                "TLS secret has no tls.crt or tls.key"
            );
            continue;
        };
        data.insert(tls_cert_file(&s.host), cert);
        data.insert(tls_key_file(&s.host), key);
        hosts.insert(s.host);
    }

    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(TLS_SECRET_NAME.to_string()),
            namespace: Some(ctx.namespace.clone()),
            labels: manifest_labels(),
            ..ObjectMeta::default()
        },
        data: Some(data),
        type_: Some("Opaque".to_string()),
        ..Secret::default()
    };

    let api: Api<Secret> = Api::namespaced(ctx.client.clone(), ctx.namespace.as_str());
    api.patch(
        TLS_SECRET_NAME,
        &PatchParams::apply(FIELD_MANAGER),
        &Patch::Apply(secret),
    )
    .await?;

    Ok(hosts)
}

pub(super) async fn cleanup_tls_secret(ctx: &Context) -> Result<(), kube::Error> {
    let api: Api<Secret> = Api::namespaced(ctx.client.clone(), ctx.namespace.as_str());
    if api.get_opt(TLS_SECRET_NAME).await?.is_some() {
        api.delete(TLS_SECRET_NAME, &DeleteParams::default())
            .await?;
        info!(
            namespace = ctx.namespace,
            name = TLS_SECRET_NAME,
            "Deleted TLS secret as no Ingress is managed"
        );
    }
    Ok(())
}

trait ExtractTls {
    fn extract_tls(&self) -> Option<(ByteString, ByteString)>;
}

impl ExtractTls for Secret {
    fn extract_tls(&self) -> Option<(ByteString, ByteString)> {
        let cert = self.data.as_ref()?.get("tls.crt")?;
        let key = self.data.as_ref()?.get("tls.key")?;

        Some((cert.clone(), key.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::ExtractTls;
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
}
