use crate::controller::FIELD_MANAGER;
use crate::controller::host_port::{CONFIG_MAP_NAME, Context, TLS_SECRET_NAME, manifest_labels};
use crate::controller::proxy::ProxyPod;
use k8s_openapi::api::apps::v1::{DaemonSet, DaemonSetSpec};
use k8s_openapi::api::core::v1::{
    ConfigMapVolumeSource, PodTemplateSpec, SecretVolumeSource, Volume,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use kube::Api;
use kube::api::{DeleteParams, Patch, PatchParams};
use std::collections::BTreeMap;
use tracing::info;

const DAEMONSET_NAME: &str = "pingress-proxy-server";

pub(super) async fn apply_daemonset(ctx: &Context) -> Result<(), kube::Error> {
    let pod = ProxyPod {
        image: ctx.proxy_server_image.as_str(),
        image_pull_secret: ctx.image_pull_secret.as_deref(),
        config_volume: Volume {
            name: CONFIG_MAP_NAME.to_string(),
            config_map: Some(ConfigMapVolumeSource {
                name: CONFIG_MAP_NAME.to_string(),
                ..ConfigMapVolumeSource::default()
            }),
            ..Volume::default()
        },
        keys_volume: Volume {
            name: TLS_SECRET_NAME.to_string(),
            secret: Some(SecretVolumeSource {
                default_mode: Some(0o400),
                secret_name: Some(TLS_SECRET_NAME.to_string()),
                ..SecretVolumeSource::default()
            }),
            ..Volume::default()
        },
        host_port: true,
        node_selector: Some(ctx.node_selector.clone()),
    };

    let daemonset = DaemonSet {
        metadata: ObjectMeta {
            name: Some(DAEMONSET_NAME.to_string()),
            namespace: Some(ctx.namespace.clone()),
            labels: manifest_labels(),
            ..ObjectMeta::default()
        },
        spec: Some(DaemonSetSpec {
            selector: LabelSelector {
                match_labels: Some(BTreeMap::from([(
                    "app.kubernetes.io/name".to_string(),
                    "pingress-proxy-server".to_string(),
                )])),
                ..LabelSelector::default()
            },
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(BTreeMap::from([(
                        "app.kubernetes.io/name".to_string(),
                        "pingress-proxy-server".to_string(),
                    )])),
                    ..ObjectMeta::default()
                }),
                spec: Some(pod.spec()),
            },
            ..DaemonSetSpec::default()
        }),
        ..DaemonSet::default()
    };

    let api: Api<DaemonSet> = Api::namespaced(ctx.client.clone(), ctx.namespace.as_str());
    api.patch(
        DAEMONSET_NAME,
        &PatchParams::apply(FIELD_MANAGER),
        &Patch::Apply(daemonset),
    )
    .await?;

    Ok(())
}

pub(super) async fn cleanup_daemonset(ctx: &Context) -> Result<(), kube::Error> {
    let api: Api<DaemonSet> = Api::namespaced(ctx.client.clone(), ctx.namespace.as_str());
    if api.get_opt(DAEMONSET_NAME).await?.is_some() {
        api.delete(DAEMONSET_NAME, &DeleteParams::default()).await?;
        info!(
            namespace = ctx.namespace,
            name = DAEMONSET_NAME,
            "Deleted DaemonSet as no Ingress is managed"
        );
    }
    Ok(())
}
