use crate::controller::host_port::{
    manifest_labels, Context, CONFIG_KEY, CONFIG_MAP_NAME, FIELD_MANAGER, SECRET_BASE_PATH,
    TLS_SECRET_NAME,
};
use k8s_openapi::api::apps::v1::{DaemonSet, DaemonSetSpec};
use k8s_openapi::api::core::v1::{
    ConfigMapVolumeSource, Container, ContainerPort, LocalObjectReference, PodSpec,
    PodTemplateSpec, SecretVolumeSource, SecurityContext, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use kube::api::{DeleteParams, Patch, PatchParams};
use kube::Api;
use std::collections::BTreeMap;

const DAEMONSET_NAME: &str = "pingress-proxy-server";

pub(super) async fn apply_daemonset(
    ctx: &Context,
    config_digest: String,
) -> Result<(), kube::Error> {
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
                    annotations: Some(BTreeMap::from([(
                        "kinorca.com/pingress-proxy-server-config-digest".to_string(),
                        config_digest,
                    )])),
                    ..ObjectMeta::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        command: Some(vec![
                            "/usr/local/bin/pingress-proxy-server".to_string(),
                            format!("--config=/etc/pingress/config/{CONFIG_KEY}"),
                            "--listen-http=0.0.0.0:8080".to_string(),
                            "--listen-https=0.0.0.0:8443".to_string(),
                        ]),
                        image: Some(ctx.proxy_server_image.clone()),
                        name: "pingress-proxy-server".to_string(),
                        ports: Some(vec![
                            ContainerPort {
                                container_port: 8080,
                                host_port: Some(80),
                                name: Some("http".to_string()),
                                protocol: Some("TCP".to_string()),
                                ..ContainerPort::default()
                            },
                            ContainerPort {
                                container_port: 8443,
                                host_port: Some(443),
                                name: Some("https".to_string()),
                                protocol: Some("TCP".to_string()),
                                ..ContainerPort::default()
                            },
                        ]),
                        liveness_probe: None,
                        readiness_probe: None,
                        startup_probe: None,
                        security_context: Some(SecurityContext {
                            read_only_root_filesystem: Some(true),
                            ..SecurityContext::default()
                        }),
                        volume_mounts: Some(vec![
                            VolumeMount {
                                mount_path: "/etc/pingress/config".to_string(),
                                name: CONFIG_MAP_NAME.to_string(),
                                read_only: Some(true),
                                ..VolumeMount::default()
                            },
                            VolumeMount {
                                mount_path: SECRET_BASE_PATH.to_string(),
                                name: TLS_SECRET_NAME.to_string(),
                                read_only: Some(true),
                                ..VolumeMount::default()
                            },
                        ]),
                        ..Container::default()
                    }],
                    image_pull_secrets: ctx.image_pull_secret.as_ref().map(|s| {
                        vec![LocalObjectReference {
                            name: Some(s.clone()),
                        }]
                    }),
                    node_selector: Some(ctx.node_selector.clone()),
                    volumes: Some(vec![
                        Volume {
                            name: TLS_SECRET_NAME.to_string(),
                            secret: Some(SecretVolumeSource {
                                default_mode: Some(0o700),
                                secret_name: Some(TLS_SECRET_NAME.to_string()),
                                ..SecretVolumeSource::default()
                            }),
                            ..Volume::default()
                        },
                        Volume {
                            name: CONFIG_MAP_NAME.to_string(),
                            config_map: Some(ConfigMapVolumeSource {
                                name: Some(CONFIG_MAP_NAME.to_string()),
                                ..ConfigMapVolumeSource::default()
                            }),
                            ..Volume::default()
                        },
                    ]),
                    ..PodSpec::default()
                }),
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
    }
    Ok(())
}
