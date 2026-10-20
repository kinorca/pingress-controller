use crate::controller::host_port::{SECRET_BASE_PATH, tls_cert_file, tls_key_file};
use k8s_openapi::api::networking::v1::{Ingress, IngressBackend, IngressSpec};
use kube::ResourceExt;
use log::warn;
use pingress_config::{Backend, HttpPath, PathRule, PingressConfiguration, Port, Tls};
use std::collections::HashSet;

pub(in crate::controller::host_port) struct TlsSecret {
    pub host: String,
    pub secret: String,
    pub namespace: String,
}

pub(super) trait GetFromIngresses {
    fn tls_secrets(&self) -> Vec<TlsSecret>;

    /// Build the proxy configuration. `tls_hosts` are the hosts whose key pair is available.
    fn config(&self, tls_hosts: &HashSet<String>) -> PingressConfiguration;
}

impl GetFromIngresses for &[Ingress] {
    fn tls_secrets(&self) -> Vec<TlsSecret> {
        let mut secrets = Vec::new();
        for ingress in self.iter() {
            let namespace = namespace_of(ingress);
            let tls = ingress.spec.iter().flat_map(|s| s.tls.iter().flatten());
            for tls in tls {
                let Some(secret) = &tls.secret_name else {
                    continue;
                };
                for host in tls.hosts.iter().flatten() {
                    secrets.push(TlsSecret {
                        host: host.to_ascii_lowercase(),
                        secret: secret.clone(),
                        namespace: namespace.clone(),
                    });
                }
            }
        }
        secrets
    }

    fn config(&self, tls_hosts: &HashSet<String>) -> PingressConfiguration {
        let mut rules = Vec::new();
        for ingress in self.iter() {
            rules.extend(ingress_to_config(ingress, tls_hosts));
        }

        // Only one default backend can be served; the first one wins
        let default_backend = self.iter().find_map(|i| {
            let backend = i.spec.as_ref()?.default_backend.as_ref()?;
            service_backend(backend, &namespace_of(i))
        });

        PingressConfiguration {
            rules,
            default_backend,
        }
    }
}

fn namespace_of(ingress: &Ingress) -> String {
    ingress.namespace().unwrap_or("default".to_string())
}

fn ingress_to_config(ingress: &Ingress, tls_hosts: &HashSet<String>) -> Vec<PathRule> {
    let Some(spec) = ingress.spec.as_ref() else {
        return Vec::new();
    };
    let namespace = namespace_of(ingress);
    let ingress_tls = ingress_tls_hosts(spec);

    let mut rules = Vec::new();
    for rule in spec.rules.iter().flatten() {
        let Some(http) = rule.http.as_ref() else {
            continue;
        };
        let host = rule.host.as_ref().map(|h| h.to_ascii_lowercase());
        let tls = host
            .as_ref()
            .filter(|h| ingress_tls.contains(*h) && tls_hosts.contains(*h))
            .map(|h| Tls {
                key: format!("{SECRET_BASE_PATH}/{}", tls_key_file(h)),
                cert: format!("{SECRET_BASE_PATH}/{}", tls_cert_file(h)),
            });

        for p in &http.paths {
            let Some(backend) = service_backend(&p.backend, &namespace) else {
                warn!(
                    "Skip path {:?} of Ingress {namespace}/{}: only services with port number are supported",
                    p.path,
                    ingress.name_any()
                );
                continue;
            };
            let path = p.path.clone().unwrap_or("/".to_string());
            rules.push(PathRule {
                host: host.clone(),
                tls: tls.clone(),
                // ImplementationSpecific is handled as Prefix
                path: if p.path_type == "Exact" {
                    HttpPath::Exact(path)
                } else {
                    HttpPath::Prefix(path)
                },
                backend,
            });
        }
    }

    rules
}

fn service_backend(backend: &IngressBackend, namespace: &str) -> Option<Backend> {
    let service = backend.service.as_ref()?;
    let port = u16::try_from(service.port.as_ref()?.number?).ok()?;
    Some(Backend::Service {
        name: service.name.clone(),
        namespace: namespace.to_string(),
        port: Port::Number(port),
    })
}

fn ingress_tls_hosts(ingress: &IngressSpec) -> HashSet<String> {
    ingress
        .tls
        .iter()
        .flatten()
        .flat_map(|t| t.hosts.iter().flatten())
        .map(|h| h.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::GetFromIngresses;
    use crate::controller::host_port::SECRET_BASE_PATH;
    use k8s_openapi::api::networking::v1::Ingress;
    use pingress_config::{Backend, HttpPath, Port};
    use serde_json::json;
    use std::collections::HashSet;

    fn tls_hosts(hosts: &[&str]) -> HashSet<String> {
        hosts.iter().map(|h| h.to_string()).collect()
    }

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
        let config = ingresses
            .as_slice()
            .config(&tls_hosts(&["secure.example.com"]));

        assert_eq!(config.rules.len(), 3);

        let r = &config.rules[0];
        assert_eq!(r.host.as_deref(), Some("secure.example.com"));
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
        assert_eq!(
            tls.cert,
            format!("{SECRET_BASE_PATH}/secure.example.com.crt")
        );

        let r = &config.rules[1];
        assert_eq!(r.host.as_deref(), Some("secure.example.com"));
        assert!(matches!(&r.path, HttpPath::Exact(p) if p == "/healthz"));
        assert!(r.tls.is_some());

        let r = &config.rules[2];
        assert_eq!(r.host.as_deref(), Some("plain.example.com"));
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
        let config = ingresses.as_slice().config(&HashSet::new());

        assert_eq!(config.rules.len(), 1);
        assert!(matches!(&config.rules[0].path, HttpPath::Prefix(p) if p == "/numbered"));
        let Backend::Service { namespace, .. } = &config.rules[0].backend;
        assert_eq!(namespace, "default");
    }

    #[test]
    fn config_from_ingress_without_spec() {
        let ingresses = [ingress(json!({ "metadata": { "name": "empty" } }))];
        let config = ingresses.as_slice().config(&HashSet::new());

        assert!(config.rules.is_empty());
    }

    #[test]
    fn tls_is_omitted_when_key_pair_is_unavailable() {
        let ingresses = [sample_ingress()];
        let config = ingresses.as_slice().config(&HashSet::new());

        assert!(config.rules.iter().all(|r| r.tls.is_none()));
    }

    #[test]
    fn rule_without_host_does_not_drop_other_rules() {
        let ingresses = [ingress(json!({
            "metadata": { "name": "mixed", "namespace": "app" },
            "spec": {
                "defaultBackend": {
                    "service": { "name": "fallback", "port": { "number": 8080 } }
                },
                "rules": [
                    { "host": "no-http.example.com" },
                    {
                        "http": {
                            "paths": [{
                                "pathType": "Prefix",
                                "backend": { "service": { "name": "any", "port": { "number": 80 } } }
                            }]
                        }
                    },
                    {
                        "host": "test.example.com",
                        "http": {
                            "paths": [{
                                "path": "/",
                                "pathType": "Prefix",
                                "backend": { "service": { "name": "web", "port": { "number": 80 } } }
                            }]
                        }
                    }
                ]
            }
        }))];
        let config = ingresses.as_slice().config(&HashSet::new());

        assert_eq!(config.rules.len(), 2);
        assert!(config.rules[0].host.is_none());
        assert!(matches!(&config.rules[0].path, HttpPath::Prefix(p) if p == "/"));
        assert_eq!(config.rules[1].host.as_deref(), Some("test.example.com"));
        let Some(Backend::Service {
            name, namespace, ..
        }) = &config.default_backend
        else {
            panic!("default backend is set");
        };
        assert_eq!(name, "fallback");
        assert_eq!(namespace, "app");
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
