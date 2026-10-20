//! Pod of pingress-proxy-server, shared by all backends

use k8s_openapi::api::core::v1::{
    Container, ContainerPort, LocalObjectReference, PodSpec, Probe, ResourceRequirements,
    SecurityContext, TCPSocketAction, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use std::collections::BTreeMap;

pub(super) const CONFIG_KEY: &str = "proxy.json";
pub(super) const CONFIG_BASE_PATH: &str = "/etc/pingress/config";
pub(super) const SECRET_BASE_PATH: &str = "/etc/pingress/keys";

pub(super) const HTTP_PORT_NAME: &str = "http";
pub(super) const HTTPS_PORT_NAME: &str = "https";

const CONTAINER_NAME: &str = "pingress-proxy-server";

pub(super) struct ProxyPod<'a> {
    pub image: &'a str,
    pub image_pull_secret: Option<&'a str>,
    /// Volume with the proxy configuration, mounted at [`CONFIG_BASE_PATH`]
    pub config_volume: Volume,
    /// Volume with the TLS key pairs, mounted at [`SECRET_BASE_PATH`]
    pub keys_volume: Volume,
    /// Bind the HTTP and HTTPS ports to the ports 80 and 443 of the node
    pub host_port: bool,
    pub node_selector: Option<BTreeMap<String, String>>,
}

fn tcp_probe(port: &str, initial_delay_seconds: i32) -> Probe {
    Probe {
        tcp_socket: Some(TCPSocketAction {
            port: IntOrString::String(port.to_string()),
            ..TCPSocketAction::default()
        }),
        initial_delay_seconds: Some(initial_delay_seconds),
        period_seconds: Some(10),
        ..Probe::default()
    }
}

impl ProxyPod<'_> {
    pub(super) fn spec(self) -> PodSpec {
        let host_port = |port| self.host_port.then_some(port);

        PodSpec {
            containers: vec![Container {
                command: Some(vec![
                    "/usr/local/bin/pingress-proxy-server".to_string(),
                    format!("--config={CONFIG_BASE_PATH}/{CONFIG_KEY}"),
                    format!("--watch={CONFIG_BASE_PATH}"),
                    format!("--watch={SECRET_BASE_PATH}"),
                    "--listen-http=0.0.0.0:8080".to_string(),
                    "--listen-https=0.0.0.0:8443".to_string(),
                    "--listen-metrics=0.0.0.0:9090".to_string(),
                ]),
                image: Some(self.image.to_string()),
                name: CONTAINER_NAME.to_string(),
                ports: Some(vec![
                    ContainerPort {
                        container_port: 8080,
                        host_port: host_port(80),
                        name: Some(HTTP_PORT_NAME.to_string()),
                        protocol: Some("TCP".to_string()),
                        ..ContainerPort::default()
                    },
                    ContainerPort {
                        container_port: 8443,
                        host_port: host_port(443),
                        name: Some(HTTPS_PORT_NAME.to_string()),
                        protocol: Some("TCP".to_string()),
                        ..ContainerPort::default()
                    },
                    ContainerPort {
                        container_port: 9090,
                        name: Some("metrics".to_string()),
                        protocol: Some("TCP".to_string()),
                        ..ContainerPort::default()
                    },
                ]),
                liveness_probe: Some(tcp_probe(HTTP_PORT_NAME, 10)),
                readiness_probe: Some(tcp_probe(HTTP_PORT_NAME, 0)),
                resources: Some(ResourceRequirements {
                    requests: Some(BTreeMap::from([
                        ("cpu".to_string(), Quantity("100m".to_string())),
                        ("memory".to_string(), Quantity("64Mi".to_string())),
                    ])),
                    ..ResourceRequirements::default()
                }),
                security_context: Some(SecurityContext {
                    read_only_root_filesystem: Some(true),
                    ..SecurityContext::default()
                }),
                volume_mounts: Some(vec![
                    VolumeMount {
                        mount_path: CONFIG_BASE_PATH.to_string(),
                        name: self.config_volume.name.clone(),
                        read_only: Some(true),
                        ..VolumeMount::default()
                    },
                    VolumeMount {
                        mount_path: SECRET_BASE_PATH.to_string(),
                        name: self.keys_volume.name.clone(),
                        read_only: Some(true),
                        ..VolumeMount::default()
                    },
                ]),
                ..Container::default()
            }],
            image_pull_secrets: self.image_pull_secret.map(|s| {
                vec![LocalObjectReference {
                    name: s.to_string(),
                }]
            }),
            node_selector: self.node_selector,
            volumes: Some(vec![self.keys_volume, self.config_volume]),
            ..PodSpec::default()
        }
    }
}
