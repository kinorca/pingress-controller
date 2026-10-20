use crate::controller::FIELD_MANAGER;
use crate::controller::host_port::{Context, TLS_SECRET_NAME, manifest_labels};
use crate::controller::ingresses::GetFromIngresses;
use crate::controller::tls::{ExtractTls, tls_cert_file, tls_key_file};
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
