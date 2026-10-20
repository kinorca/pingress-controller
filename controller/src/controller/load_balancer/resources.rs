use crate::controller::FIELD_MANAGER;
use crate::controller::ingresses::{GetFromIngresses, TlsSecret};
use crate::controller::load_balancer::{Context, manifest_labels, selector_labels};
use crate::controller::proxy::{CONFIG_KEY, HTTP_PORT_NAME, HTTPS_PORT_NAME, ProxyPod};
use crate::controller::tls::{TLS_CERT_KEY, TLS_PRIVATE_KEY_KEY, tls_cert_file, tls_key_file};
use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec};
use k8s_openapi::api::core::v1::{
    ConfigMap, ConfigMapVolumeSource, EmptyDirVolumeSource, KeyToPath, PodTemplateSpec,
    ProjectedVolumeSource, SecretProjection, Service, ServicePort, ServiceSpec, Volume,
    VolumeProjection,
};
use k8s_openapi::api::networking::v1::Ingress;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta, OwnerReference};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{DeleteParams, Patch, PatchParams};
use kube::{Api, Resource, ResourceExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Debug;
use tracing::{info, warn};

const CONFIG_VOLUME_NAME: &str = "config";
const KEYS_VOLUME_NAME: &str = "keys";

/// Resources generated for one Ingress, all named `name` in the namespace of the Ingress
pub(super) struct Resources<'a> {
    pub ingress: &'a Ingress,
    pub namespace: &'a str,
    pub name: &'a str,
    /// TLS secrets whose key pair is available, at most one per host
    pub tls: &'a [TlsSecret],
}

impl Resources<'_> {
    fn metadata(&self) -> ObjectMeta {
        ObjectMeta {
            name: Some(self.name.to_string()),
            namespace: Some(self.namespace.to_string()),
            labels: Some(manifest_labels(self.name)),
            owner_references: owner_reference(self.ingress).map(|r| vec![r]),
            ..ObjectMeta::default()
        }
    }

    pub(super) fn config_map(&self) -> Result<ConfigMap, serde_json::Error> {
        let tls_hosts: HashSet<String> = self.tls.iter().map(|t| t.host.clone()).collect();
        let config = std::slice::from_ref(self.ingress).config(&tls_hosts);

        Ok(ConfigMap {
            metadata: self.metadata(),
            data: Some(BTreeMap::from([(
                CONFIG_KEY.to_string(),
                serde_json::to_string(&config)?,
            )])),
            ..ConfigMap::default()
        })
    }

    /// The TLS secrets of the Ingress are projected directly into the proxy, so that the
    /// controller does not need to copy them
    fn keys_volume(&self) -> Volume {
        if self.tls.is_empty() {
            return Volume {
                name: KEYS_VOLUME_NAME.to_string(),
                empty_dir: Some(EmptyDirVolumeSource::default()),
                ..Volume::default()
            };
        }

        let mut secrets: BTreeMap<&str, Vec<KeyToPath>> = BTreeMap::new();
        for t in self.tls {
            secrets.entry(t.secret.as_str()).or_default().extend([
                KeyToPath {
                    key: TLS_CERT_KEY.to_string(),
                    path: tls_cert_file(&t.host),
                    mode: None,
                },
                KeyToPath {
                    key: TLS_PRIVATE_KEY_KEY.to_string(),
                    path: tls_key_file(&t.host),
                    mode: None,
                },
            ]);
        }

        Volume {
            name: KEYS_VOLUME_NAME.to_string(),
            projected: Some(ProjectedVolumeSource {
                default_mode: Some(0o400),
                sources: Some(
                    secrets
                        .into_iter()
                        .map(|(secret, items)| VolumeProjection {
                            secret: Some(SecretProjection {
                                name: secret.to_string(),
                                items: Some(items),
                                // A deleted secret must not keep the proxy from starting
                                optional: Some(true),
                            }),
                            ..VolumeProjection::default()
                        })
                        .collect(),
                ),
            }),
            ..Volume::default()
        }
    }

    pub(super) fn deployment(&self, ctx: &Context) -> Deployment {
        let pod = ProxyPod {
            image: ctx.options.proxy_server_image.as_str(),
            image_pull_secret: ctx.options.image_pull_secret.as_deref(),
            config_volume: Volume {
                name: CONFIG_VOLUME_NAME.to_string(),
                config_map: Some(ConfigMapVolumeSource {
                    name: self.name.to_string(),
                    ..ConfigMapVolumeSource::default()
                }),
                ..Volume::default()
            },
            keys_volume: self.keys_volume(),
            host_port: false,
            node_selector: None,
        };

        Deployment {
            metadata: self.metadata(),
            spec: Some(DeploymentSpec {
                replicas: Some(ctx.options.replicas),
                selector: LabelSelector {
                    match_labels: Some(selector_labels(self.name)),
                    ..LabelSelector::default()
                },
                template: PodTemplateSpec {
                    metadata: Some(ObjectMeta {
                        labels: Some(selector_labels(self.name)),
                        ..ObjectMeta::default()
                    }),
                    spec: Some(pod.spec()),
                },
                ..DeploymentSpec::default()
            }),
            ..Deployment::default()
        }
    }

    pub(super) fn service(&self, ctx: &Context) -> Service {
        let port = |name: &str, port| ServicePort {
            name: Some(name.to_string()),
            port,
            protocol: Some("TCP".to_string()),
            target_port: Some(IntOrString::String(name.to_string())),
            ..ServicePort::default()
        };

        Service {
            metadata: self.metadata(),
            spec: Some(ServiceSpec {
                type_: Some("LoadBalancer".to_string()),
                load_balancer_class: ctx.options.load_balancer_class.clone(),
                selector: Some(selector_labels(self.name)),
                ports: Some(vec![port(HTTP_PORT_NAME, 80), port(HTTPS_PORT_NAME, 443)]),
                ..ServiceSpec::default()
            }),
            ..Service::default()
        }
    }
}

fn owner_reference(ingress: &Ingress) -> Option<OwnerReference> {
    let mut owner = ingress.controller_owner_ref(&())?;
    // The Ingress is not deleted until the generated resources are deleted by the finalizer, so
    // the garbage collector does not need to block it
    owner.block_owner_deletion = None;
    Some(owner)
}

/// Apply the resources and return the Service, which has the current load balancer status
pub(super) async fn apply(
    ctx: &Context,
    resources: &Resources<'_>,
) -> Result<Service, kube::Error> {
    let config_map = resources.config_map().map_err(kube::Error::SerdeError)?;
    apply_one(ctx, resources, config_map).await?;
    apply_one(ctx, resources, resources.deployment(ctx)).await?;
    apply_one(ctx, resources, resources.service(ctx)).await
}

async fn apply_one<K>(ctx: &Context, resources: &Resources<'_>, object: K) -> Result<K, kube::Error>
where
    K: Resource<DynamicType = (), Scope = k8s_openapi::NamespaceResourceScope>
        + Clone
        + Debug
        + Serialize
        + DeserializeOwned,
{
    let api: Api<K> = Api::namespaced(ctx.client.clone(), resources.namespace);
    api.patch(
        resources.name,
        &PatchParams::apply(FIELD_MANAGER),
        &Patch::Apply(object),
    )
    .await
}

/// Delete the resources generated for the Ingress. Resources that are not owned by the Ingress
/// are left untouched.
pub(super) async fn cleanup(
    ctx: &Context,
    ingress: &Ingress,
    namespace: &str,
    name: &str,
) -> Result<(), kube::Error> {
    cleanup_one::<Service>(ctx, ingress, namespace, name).await?;
    cleanup_one::<Deployment>(ctx, ingress, namespace, name).await?;
    cleanup_one::<ConfigMap>(ctx, ingress, namespace, name).await?;
    Ok(())
}

async fn cleanup_one<K>(
    ctx: &Context,
    ingress: &Ingress,
    namespace: &str,
    name: &str,
) -> Result<(), kube::Error>
where
    K: Resource<DynamicType = (), Scope = k8s_openapi::NamespaceResourceScope>
        + Clone
        + Debug
        + DeserializeOwned,
{
    let api: Api<K> = Api::namespaced(ctx.client.clone(), namespace);
    let Some(object) = api.get_opt(name).await? else {
        return Ok(());
    };

    let owned = ingress.uid().is_some_and(|uid| {
        object
            .owner_references()
            .iter()
            .any(|r| r.uid == uid && r.controller == Some(true))
    });
    if !owned {
        warn!(
            kind = K::kind(&()).as_ref(),
            resource_namespace = namespace,
            resource_name = name,
            "Resource is not owned by the Ingress, leaving it"
        );
        return Ok(());
    }

    api.delete(name, &DeleteParams::default()).await?;
    info!(
        kind = K::kind(&()).as_ref(),
        resource_namespace = namespace,
        resource_name = name,
        "Deleted resource of the Ingress"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Resources;
    use crate::controller::Metrics;
    use crate::controller::ingresses::TlsSecret;
    use crate::controller::load_balancer::{Context, LoadBalancerOptions};
    use crate::controller::proxy::CONFIG_KEY;
    use k8s_openapi::api::networking::v1::Ingress;
    use pingress_config::PingressConfiguration;
    use serde_json::json;
    use std::sync::Arc;

    fn ingress() -> Ingress {
        serde_json::from_value(json!({
            "metadata": { "name": "web", "namespace": "app", "uid": "1234" },
            "spec": {
                "tls": [{ "hosts": ["secure.example.com"], "secretName": "secure-tls" }],
                "rules": [{
                    "host": "secure.example.com",
                    "http": {
                        "paths": [{
                            "path": "/",
                            "pathType": "Prefix",
                            "backend": { "service": { "name": "web", "port": { "number": 80 } } }
                        }]
                    }
                }]
            }
        }))
        .unwrap()
    }

    fn tls() -> Vec<TlsSecret> {
        vec![TlsSecret {
            host: "secure.example.com".to_string(),
            secret: "secure-tls".to_string(),
            namespace: "app".to_string(),
        }]
    }

    /// The client is never used by the builders
    async fn context() -> Context {
        let config = kube::Config::new("http://127.0.0.1:1".parse().unwrap());
        Context::new(
            kube::Client::try_from(config).unwrap(),
            LoadBalancerOptions {
                image_pull_secret: None,
                proxy_server_image: "proxy:test".to_string(),
                replicas: 3,
                load_balancer_class: Some("example.com/lb".to_string()),
            },
            Arc::new(Metrics::new()),
        )
    }

    #[test]
    fn config_map() {
        let ingress = ingress();
        let tls = tls();
        let resources = Resources {
            ingress: &ingress,
            namespace: "app",
            name: "pingress-web",
            tls: &tls,
        };

        let config_map = resources.config_map().unwrap();
        assert_eq!(config_map.metadata.name.as_deref(), Some("pingress-web"));
        assert_eq!(config_map.metadata.namespace.as_deref(), Some("app"));
        let owner = &config_map.metadata.owner_references.unwrap()[0];
        assert_eq!(owner.kind, "Ingress");
        assert_eq!(owner.uid, "1234");
        assert_eq!(owner.controller, Some(true));

        let config: PingressConfiguration =
            serde_json::from_str(&config_map.data.unwrap()[CONFIG_KEY]).unwrap();
        assert_eq!(config.rules.len(), 1);
        assert!(config.rules[0].tls.is_some());
    }

    #[tokio::test]
    async fn deployment_projects_tls_secrets() {
        let ctx = context().await;
        let ingress = ingress();
        let tls = tls();
        let resources = Resources {
            ingress: &ingress,
            namespace: "app",
            name: "pingress-web",
            tls: &tls,
        };

        let deployment = resources.deployment(&ctx);
        let spec = deployment.spec.unwrap();
        assert_eq!(spec.replicas, Some(3));
        assert_eq!(
            spec.selector.match_labels,
            spec.template.metadata.unwrap().labels
        );

        let pod = spec.template.spec.unwrap();
        let ports = pod.containers[0].ports.as_ref().unwrap();
        assert!(ports.iter().all(|p| p.host_port.is_none()));

        let volumes = pod.volumes.unwrap();
        let config = volumes.iter().find(|v| v.name == "config").unwrap();
        assert_eq!(config.config_map.as_ref().unwrap().name, "pingress-web");
        let keys = volumes.iter().find(|v| v.name == "keys").unwrap();
        let sources = keys.projected.as_ref().unwrap().sources.as_ref().unwrap();
        assert_eq!(sources.len(), 1);
        let secret = sources[0].secret.as_ref().unwrap();
        assert_eq!(secret.name, "secure-tls");
        assert_eq!(secret.optional, Some(true));
        let paths: Vec<&str> = secret
            .items
            .iter()
            .flatten()
            .map(|i| i.path.as_str())
            .collect();
        assert_eq!(paths, ["secure.example.com.crt", "secure.example.com.key"]);
    }

    #[tokio::test]
    async fn deployment_without_tls() {
        let ctx = context().await;
        let ingress = ingress();
        let resources = Resources {
            ingress: &ingress,
            namespace: "app",
            name: "pingress-web",
            tls: &[],
        };

        let pod = resources
            .deployment(&ctx)
            .spec
            .unwrap()
            .template
            .spec
            .unwrap();
        let volumes = pod.volumes.unwrap();
        let keys = volumes.iter().find(|v| v.name == "keys").unwrap();
        assert!(keys.empty_dir.is_some());
    }

    #[tokio::test]
    async fn service() {
        let ctx = context().await;
        let ingress = ingress();
        let resources = Resources {
            ingress: &ingress,
            namespace: "app",
            name: "pingress-web",
            tls: &[],
        };

        let spec = resources.service(&ctx).spec.unwrap();
        assert_eq!(spec.type_.as_deref(), Some("LoadBalancer"));
        assert_eq!(spec.load_balancer_class.as_deref(), Some("example.com/lb"));
        let ports: Vec<(i32, &str)> = spec
            .ports
            .iter()
            .flatten()
            .map(|p| (p.port, p.name.as_deref().unwrap()))
            .collect();
        assert_eq!(ports, [(80, "http"), (443, "https")]);
        assert_eq!(
            spec.selector.unwrap()["app.kubernetes.io/instance"],
            "pingress-web"
        );
    }
}
