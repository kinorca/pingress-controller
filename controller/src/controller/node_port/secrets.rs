use crate::controller::node_port::ingresses::{GetFromIngresses, TlsSecret};
use crate::controller::node_port::{manifest_labels, Context, FIELD_MANAGER, TLS_SECRET_NAME};
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::api::networking::v1::Ingress;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::ByteString;
use kube::api::{DeleteParams, Patch, PatchParams};
use kube::{Api, Client};
use std::collections::BTreeMap;

pub(super) async fn apply_tls_secrets(
    ctx: &Context,
    ingresses: &[Ingress],
) -> Result<(), kube::Error> {
    let tls_secrets = {
        let secrets = ingresses.tls_secrets();

        let mut ss = BTreeMap::new();
        for s in secrets {
            let secret = load_secrets(ctx.client.clone(), &s).await?;
            if let Some((cert, key)) = secret.extract_tls() {
                ss.insert(format!("{}.crt", s.host), cert);
                ss.insert(format!("{}.key", s.host), key);
            }
        }

        ss
    };

    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(TLS_SECRET_NAME.to_string()),
            namespace: Some(ctx.namespace.clone()),
            labels: manifest_labels(),
            ..ObjectMeta::default()
        },
        data: Some(tls_secrets),
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

    Ok(())
}

pub(super) async fn cleanup_tls_secret(ctx: &Context) -> Result<(), kube::Error> {
    let api: Api<Secret> = Api::namespaced(ctx.client.clone(), ctx.namespace.as_str());
    api.delete(TLS_SECRET_NAME, &DeleteParams::default())
        .await?;
    Ok(())
}

async fn load_secrets(client: Client, tls_secret: &TlsSecret) -> Result<Secret, kube::Error> {
    let api: Api<Secret> = Api::namespaced(client, tls_secret.namespace.as_str());
    api.get(tls_secret.secret.as_str()).await
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
