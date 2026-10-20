use crate::proxy_map::detail::{ExactProxyEntry, RegexProxyEntry};
use pingress_config::{Backend, HttpPath, PingressConfiguration, Port};
use regex::Regex;
use std::collections::HashMap;

pub(crate) struct ProxyMap {
    exact_proxy_entries: HashMap<String, ExactProxyEntry>,
    regex_proxy_entries: Vec<RegexProxyEntry>,
}

impl ProxyMap {
    pub(crate) fn get_backend(&self, host: &str, path: &str) -> Option<String> {
        if let Some(entry) = self.exact_proxy_entries.get(host) {
            if entry.path.is_match(path) {
                return Some(entry.backend_host.clone());
            }
        }

        for entry in &self.regex_proxy_entries {
            if entry.pattern.is_match(host) && entry.path.is_match(path) {
                return Some(entry.backend_host.clone());
            }
        }

        None
    }
}

impl From<PingressConfiguration> for ProxyMap {
    fn from(value: PingressConfiguration) -> Self {
        let rules: Vec<(String, Backend, HttpPath)> = value
            .rules
            .into_iter()
            .map(|r| (r.host.clone(), r.backend.clone(), r.path.clone()))
            .collect();
        let mut exact = HashMap::new();
        let mut regex = Vec::new();

        for (host, backend, path) in rules {
            let backend_host = match backend {
                Backend::Service {
                    name,
                    namespace,
                    port,
                } => format!(
                    "{name}.{namespace}:{}",
                    match port {
                        Port::Number(n) => n,
                    }
                ),
            };

            if host.contains("*") {
                regex.push(RegexProxyEntry {
                    pattern: Regex::new(host.replace("*", ".+").as_str()).unwrap(),
                    path,
                    backend_host,
                });
            } else {
                exact.insert(host, ExactProxyEntry { path, backend_host });
            }
        }

        Self {
            exact_proxy_entries: exact,
            regex_proxy_entries: regex,
        }
    }
}

trait IsMatch {
    fn is_match(&self, haystack: &str) -> bool;
}

impl IsMatch for HttpPath {
    fn is_match(&self, haystack: &str) -> bool {
        match self {
            HttpPath::Prefix(prefix) => haystack.starts_with(prefix),
            HttpPath::Exact(exact) => haystack == exact,
        }
    }
}

mod detail {
    use pingress_config::HttpPath;
    use regex::Regex;

    pub(super) struct RegexProxyEntry {
        pub(super) pattern: Regex,
        pub(super) path: HttpPath,
        pub(super) backend_host: String,
    }

    pub(super) struct ExactProxyEntry {
        pub(super) path: HttpPath,
        pub(super) backend_host: String,
    }
}

#[cfg(test)]
mod tests {
    use super::ProxyMap;
    use pingress_config::{Backend, HttpPath, PathRule, PingressConfiguration, Port};

    fn rule(host: &str, path: HttpPath, name: &str, port: u16) -> PathRule {
        PathRule {
            host: host.to_string(),
            tls: None,
            path,
            backend: Backend::Service {
                name: name.to_string(),
                namespace: "default".to_string(),
                port: Port::Number(port),
            },
        }
    }

    fn proxy_map(rules: Vec<PathRule>) -> ProxyMap {
        ProxyMap::from(PingressConfiguration { rules })
    }

    #[test]
    fn exact_host_with_prefix_path() {
        let map = proxy_map(vec![rule(
            "test.example.com",
            HttpPath::Prefix("/api".to_string()),
            "backend",
            80,
        )]);

        assert_eq!(
            map.get_backend("test.example.com", "/api"),
            Some("backend.default:80".to_string())
        );
        assert_eq!(
            map.get_backend("test.example.com", "/api/v1/users"),
            Some("backend.default:80".to_string())
        );
        assert_eq!(map.get_backend("test.example.com", "/"), None);
    }

    #[test]
    fn exact_host_with_exact_path() {
        let map = proxy_map(vec![rule(
            "test.example.com",
            HttpPath::Exact("/v1/users".to_string()),
            "backend-api",
            8080,
        )]);

        assert_eq!(
            map.get_backend("test.example.com", "/v1/users"),
            Some("backend-api.default:8080".to_string())
        );
        assert_eq!(map.get_backend("test.example.com", "/v1/users/1"), None);
        assert_eq!(map.get_backend("test.example.com", "/v1"), None);
    }

    #[test]
    fn unknown_host() {
        let map = proxy_map(vec![rule(
            "test.example.com",
            HttpPath::Prefix("/".to_string()),
            "backend",
            80,
        )]);

        assert_eq!(map.get_backend("other.example.com", "/"), None);
    }

    #[test]
    fn wildcard_host() {
        let map = proxy_map(vec![rule(
            "*.example.net",
            HttpPath::Prefix("/".to_string()),
            "backend",
            80,
        )]);

        assert_eq!(
            map.get_backend("foo.example.net", "/"),
            Some("backend.default:80".to_string())
        );
        assert_eq!(
            map.get_backend("bar.example.net", "/index.html"),
            Some("backend.default:80".to_string())
        );
        assert_eq!(map.get_backend("example.net", "/"), None);
        assert_eq!(map.get_backend("foo.example.com", "/"), None);
    }

    #[test]
    fn exact_host_takes_precedence_over_wildcard() {
        let map = proxy_map(vec![
            rule(
                "*.example.com",
                HttpPath::Prefix("/".to_string()),
                "wildcard",
                80,
            ),
            rule(
                "test.example.com",
                HttpPath::Prefix("/".to_string()),
                "exact",
                80,
            ),
        ]);

        assert_eq!(
            map.get_backend("test.example.com", "/"),
            Some("exact.default:80".to_string())
        );
        assert_eq!(
            map.get_backend("other.example.com", "/"),
            Some("wildcard.default:80".to_string())
        );
    }

    #[test]
    fn falls_back_to_wildcard_when_exact_path_does_not_match() {
        let map = proxy_map(vec![
            rule(
                "test.example.com",
                HttpPath::Exact("/exact".to_string()),
                "exact",
                80,
            ),
            rule(
                "*.example.com",
                HttpPath::Prefix("/".to_string()),
                "wildcard",
                80,
            ),
        ]);

        assert_eq!(
            map.get_backend("test.example.com", "/exact"),
            Some("exact.default:80".to_string())
        );
        assert_eq!(
            map.get_backend("test.example.com", "/other"),
            Some("wildcard.default:80".to_string())
        );
    }

    #[test]
    fn empty_configuration() {
        let map = proxy_map(vec![]);

        assert_eq!(map.get_backend("test.example.com", "/"), None);
    }
}
