use crate::controller::FIELD_MANAGER;
use crate::controller::host_port::{CONFIG_MAP_NAME, Context, manifest_labels};
use crate::controller::ingresses::GetFromIngresses;
use crate::controller::proxy::CONFIG_KEY;
use k8s_openapi::api::core::v1::ConfigMap;
use k8s_openapi::api::networking::v1::Ingress;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Api;
use kube::api::{DeleteParams, Patch, PatchParams};
use std::collections::{BTreeMap, HashSet};
use tracing::info;

pub(super) async fn apply_config_map(
    ctx: &Context,
    ingresses: &[Ingress],
    tls_hosts: &HashSet<String>,
) -> Result<(), kube::Error> {
    let config = ingresses.config(tls_hosts);

    let config = serde_json::to_string(&config).map_err(kube::Error::SerdeError)?;

    let config_map = ConfigMap {
        metadata: ObjectMeta {
            name: Some(CONFIG_MAP_NAME.to_string()),
            namespace: Some(ctx.namespace.clone()),
            labels: manifest_labels(),
            ..ObjectMeta::default()
        },
        data: Some(BTreeMap::from([(CONFIG_KEY.to_string(), config)])),
        ..ConfigMap::default()
    };

    let api: Api<ConfigMap> = Api::namespaced(ctx.client.clone(), ctx.namespace.as_str());
    api.patch(
        CONFIG_MAP_NAME,
        &PatchParams::apply(FIELD_MANAGER),
        &Patch::Apply(config_map),
    )
    .await?;

    Ok(())
}

pub(super) async fn cleanup_config_map(ctx: &Context) -> Result<(), kube::Error> {
    let api: Api<ConfigMap> = Api::namespaced(ctx.client.clone(), ctx.namespace.as_str());
    if api.get_opt(CONFIG_MAP_NAME).await?.is_some() {
        api.delete(CONFIG_MAP_NAME, &DeleteParams::default())
            .await?;
        info!(
            namespace = ctx.namespace,
            name = CONFIG_MAP_NAME,
            "Deleted ConfigMap as no Ingress is managed"
        );
    }

    Ok(())
}
