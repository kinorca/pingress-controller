use crate::controller::node_port::ingresses::GetFromIngresses;
use crate::controller::node_port::{
    manifest_labels, Context, CONFIG_KEY, CONFIG_MAP_NAME, FIELD_MANAGER,
};
use k8s_openapi::api::core::v1::ConfigMap;
use k8s_openapi::api::networking::v1::Ingress;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{DeleteParams, Patch, PatchParams};
use kube::Api;
use sha2::{Digest, Sha512};
use std::collections::BTreeMap;

pub(super) async fn apply_config_map(
    ctx: &Context,
    ingresses: &[Ingress],
) -> Result<String, kube::Error> {
    let config = ingresses.config();

    let config = serde_json::to_string(&config).map_err(kube::Error::SerdeError)?;

    let digest = Sha512::digest(config.as_bytes());
    let digest = hex::encode(digest.as_slice());

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

    Ok(digest)
}

pub(super) async fn cleanup_config_map(ctx: &Context) -> Result<(), kube::Error> {
    let api: Api<ConfigMap> = Api::namespaced(ctx.client.clone(), ctx.namespace.as_str());
    api.delete(CONFIG_MAP_NAME, &DeleteParams::default())
        .await?;

    Ok(())
}
