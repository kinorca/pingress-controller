use crate::controller::host_port::SECRET_BASE_PATH;
use k8s_openapi::api::networking::v1::{Ingress, IngressSpec};
use kube::ResourceExt;
use pingress_config::{Backend, HttpPath, PathRule, PingressConfiguration, Port, Tls};
use std::collections::HashSet;

pub(in crate::controller::host_port) struct TlsSecret {
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

#[cfg(test)]
mod tests {
    use super::GetFromIngresses;
    use crate::controller::host_port::SECRET_BASE_PATH;
    use k8s_openapi::api::networking::v1::Ingress;
    use pingress_config::{Backend, HttpPath, Port};
    use serde_json::json;

    fn ingress(value: serde_json::Value) -> Ingress {
        serde_json::from_value(value).expect("valid ingress")
    }

    fn sample_ingress() -> Ingress {
        ingress(json!({
            "apiVersion": "networking.k8s.io/v1",
            "kind": "Ingress",
            "metadata": { "name": "sample", "namespace": "app" },
            "spec": {
                "ingressClassName": "pingress",
                "tls": [
                    { "hosts": ["secure.example.com"], "secretName": "secure-tls" }
                ],
                "rules": [
                    {
                        "host": "secure.example.com",
                        "http": {
                            "paths": [
                                {
                                    "path": "/",
                                    "pathType": "Prefix",
                                    "backend": {
                                        "service": { "name": "web", "port": { "number": 80 } }
                                    }
                                },
                                {
                                    "path": "/healthz",
                                    "pathType": "Exact",
                                    "backend": {
                                        "service": { "name": "health", "port": { "number": 8080 } }
                                    }
                                }
                            ]
                        }
                    },
                    {
                        "host": "plain.example.com",
                        "http": {
                            "paths": [
                                {
                                    "path": "/api",
                                    "pathType": "ImplementationSpecific",
                                    "backend": {
                                        "service": { "name": "api", "port": { "number": 3000 } }
                                    }
                                }
                            ]
                        }
                    }
                ]
            }
        }))
    }

    #[test]
    fn config_from_ingress() {
        let ingresses = [sample_ingress()];
        let config = ingresses.as_slice().config();

        assert_eq!(config.rules.len(), 3);

        let r = &config.rules[0];
        assert_eq!(r.host, "secure.example.com");
        assert!(matches!(&r.path, HttpPath::Prefix(p) if p == "/"));
        let Backend::Service {
            name,
            namespace,
            port,
        } = &r.backend;
        assert_eq!(name, "web");
        assert_eq!(namespace, "app");
        assert!(matches!(port, Port::Number(80)));
        let tls = r.tls.as_ref().expect("tls is configured");
        assert_eq!(
            tls.key,
            format!("{SECRET_BASE_PATH}/secure.example.com.key")
        );

        let r = &config.rules[1];
        assert_eq!(r.host, "secure.example.com");
        assert!(matches!(&r.path, HttpPath::Exact(p) if p == "/healthz"));
        assert!(r.tls.is_some());

        let r = &config.rules[2];
        assert_eq!(r.host, "plain.example.com");
        assert!(matches!(&r.path, HttpPath::Prefix(p) if p == "/api"));
        assert!(r.tls.is_none());
        let Backend::Service { name, port, .. } = &r.backend;
        assert_eq!(name, "api");
        assert!(matches!(port, Port::Number(3000)));
    }

    #[test]
    fn config_skips_paths_without_service_port_number() {
        let ingresses = [ingress(json!({
            "metadata": { "name": "named-port" },
            "spec": {
                "rules": [
                    {
                        "host": "test.example.com",
                        "http": {
                            "paths": [
                                {
                                    "path": "/named",
                                    "pathType": "Prefix",
                                    "backend": {
                                        "service": { "name": "web", "port": { "name": "http" } }
                                    }
                                },
                                {
                                    "path": "/numbered",
                                    "pathType": "Prefix",
                                    "backend": {
                                        "service": { "name": "web", "port": { "number": 80 } }
                                    }
                                }
                            ]
                        }
                    }
                ]
            }
        }))];
        let config = ingresses.as_slice().config();

        assert_eq!(config.rules.len(), 1);
        assert!(matches!(&config.rules[0].path, HttpPath::Prefix(p) if p == "/numbered"));
        let Backend::Service { namespace, .. } = &config.rules[0].backend;
        assert_eq!(namespace, "default");
    }

    #[test]
    fn config_from_ingress_without_spec() {
        let ingresses = [ingress(json!({ "metadata": { "name": "empty" } }))];
        let config = ingresses.as_slice().config();

        assert!(config.rules.is_empty());
    }

    #[test]
    fn tls_secrets_from_ingress() {
        let ingresses = [ingress(json!({
            "metadata": { "name": "multi", "namespace": "app" },
            "spec": {
                "tls": [
                    { "hosts": ["a.example.com", "b.example.com"], "secretName": "ab-tls" },
                    { "hosts": ["c.example.com"] }
                ]
            }
        }))];
        let secrets = ingresses.as_slice().tls_secrets();

        assert_eq!(secrets.len(), 2);
        assert_eq!(secrets[0].host, "a.example.com");
        assert_eq!(secrets[1].host, "b.example.com");
        for s in &secrets {
            assert_eq!(s.secret, "ab-tls");
            assert_eq!(s.namespace, "app");
        }
    }

    #[test]
    fn tls_secrets_default_namespace() {
        let ingresses = [ingress(json!({
            "metadata": { "name": "no-namespace" },
            "spec": {
                "tls": [{ "hosts": ["a.example.com"], "secretName": "a-tls" }]
            }
        }))];
        let secrets = ingresses.as_slice().tls_secrets();

        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].namespace, "default");
    }
}
