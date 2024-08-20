use crate::controller::node_port::SECRET_BASE_PATH;
use k8s_openapi::api::networking::v1::{Ingress, IngressSpec};
use kube::ResourceExt;
use pingress_config::{Backend, HttpPath, PathRule, PingressConfiguration, Port, Tls};
use std::collections::HashSet;

pub(in crate::controller::node_port) struct TlsSecret {
    pub host: String,
    pub secret: String,
    pub namespace: String,
}

pub(super) trait GetFromIngresses {
    fn tls_secrets(&self) -> Vec<TlsSecret>;

    fn config(&self) -> PingressConfiguration;
}

impl GetFromIngresses for &[Ingress] {
    fn tls_secrets(&self) -> Vec<TlsSecret> {
        self.iter()
            .filter_map(|ingress| {
                ingress
                    .spec
                    .as_ref()
                    .map(|i| (i, ingress.namespace().unwrap_or("default".to_string())))
            })
            .filter_map(|(ingress, namespace)| {
                ingress
                    .tls
                    .as_ref()
                    .map(move |t| t.iter().map(move |t| (t, namespace.clone())))
            })
            .flatten()
            .filter_map(|(tls, namespace)| {
                tls.secret_name.clone().and_then(move |secret| {
                    tls.hosts.clone().map(move |hosts| {
                        hosts.into_iter().map(move |host| TlsSecret {
                            host,
                            secret: secret.clone(),
                            namespace: namespace.clone(),
                        })
                    })
                })
            })
            .flatten()
            .collect()
    }

    fn config(&self) -> PingressConfiguration {
        let mut rules = Vec::new();
        for ingress in self.iter() {
            if let Some(rs) = ingress_to_config(ingress) {
                rules.extend(rs);
            }
        }
        PingressConfiguration { rules }
    }
}

fn ingress_to_config(ingress: &Ingress) -> Option<Vec<PathRule>> {
    let spec = ingress.spec.as_ref()?;

    let tls = ingress_to_tls_map(spec).unwrap_or_default();

    let mut rules = Vec::new();
    for path in spec.rules.as_ref()? {
        let host = path.host.as_ref()?;
        let rule = path.http.as_ref()?;
        let tls = if tls.contains(host) {
            Some(Tls {
                key: format!("{SECRET_BASE_PATH}/{host}.key"),
                cert: format!("{SECRET_BASE_PATH}/{host}.cert"),
            })
        } else {
            None
        };
        let rs = rule.paths.iter().filter_map(|p| {
            Some(PathRule {
                host: host.to_string(),
                tls: tls.clone(),
                path: if p.path_type == "Exact" {
                    HttpPath::Exact(p.path.clone().unwrap_or_default())
                } else {
                    HttpPath::Prefix(p.path.clone().unwrap_or_default())
                },
                backend: {
                    let service = p.backend.service.as_ref()?;
                    Backend::Service {
                        name: service.name.clone(),
                        namespace: ingress.namespace().unwrap_or("default".to_string()),
                        port: Port::Number(service.port.as_ref()?.number? as u16),
                    }
                },
            })
        });
        rules.extend(rs);
    }

    Some(rules)
}

fn ingress_to_tls_map(ingress: &IngressSpec) -> Option<HashSet<String>> {
    Some(
        ingress
            .tls
            .as_ref()?
            .iter()
            .filter_map(|t| t.hosts.clone())
            .flatten()
            .collect(),
    )
}
