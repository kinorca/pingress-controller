use pingress_config::{Backend, HttpPath, PingressConfiguration, Port};
use std::cmp::Reverse;
use std::collections::HashMap;

pub(crate) struct ProxyMap {
    /// Routes keyed by exact host name
    exact_routes: HashMap<String, Vec<Route>>,
    /// Routes keyed by the suffix of a wildcard host (`*.example.com` → `example.com`)
    wildcard_routes: HashMap<String, Vec<Route>>,
    /// Routes of rules without host
    any_host_routes: Vec<Route>,
    default_backend: Option<String>,
}

struct Route {
    path: HttpPath,
    backend_host: String,
}

impl ProxyMap {
    /// Find a backend for the request.
    ///
    /// Exact hosts take precedence over wildcard hosts, which take precedence over rules without
    /// host. Within the same host, the longest path wins and `Exact` wins over `Prefix`.
    pub(crate) fn get_backend(&self, host: &str, path: &str) -> Option<&str> {
        let host = host.to_ascii_lowercase();

        let exact = self.exact_routes.get(&host);
        let wildcard = wildcard_suffix(&host).and_then(|s| self.wildcard_routes.get(s));

        exact
            .into_iter()
            .chain(wildcard)
            .chain([&self.any_host_routes])
            .flatten()
            .find(|r| r.path.is_match(path))
            .map(|r| r.backend_host.as_str())
            .or(self.default_backend.as_deref())
    }

    /// Number of routes, including the default backend
    pub(crate) fn len(&self) -> usize {
        self.exact_routes
            .values()
            .chain(self.wildcard_routes.values())
            .map(Vec::len)
            .sum::<usize>()
            + self.any_host_routes.len()
            + usize::from(self.default_backend.is_some())
    }
}

/// The part that a single-label wildcard host must match (`foo.example.com` → `example.com`)
pub(crate) fn wildcard_suffix(host: &str) -> Option<&str> {
    let (_, suffix) = host.split_once('.')?;
    Some(suffix)
}

fn backend_host(backend: Backend) -> String {
    match backend {
        Backend::Service {
            name,
            namespace,
            port: Port::Number(port),
        } => format!("{name}.{namespace}.svc:{port}"),
    }
}

impl From<PingressConfiguration> for ProxyMap {
    fn from(value: PingressConfiguration) -> Self {
        let mut exact: HashMap<String, Vec<Route>> = HashMap::new();
        let mut wildcard: HashMap<String, Vec<Route>> = HashMap::new();
        let mut any_host = Vec::new();

        for rule in value.rules {
            let route = Route {
                path: rule.path,
                backend_host: backend_host(rule.backend),
            };

            match rule.host.map(|h| h.to_ascii_lowercase()) {
                None => any_host.push(route),
                Some(host) => match host.strip_prefix("*.") {
                    Some(suffix) => wildcard.entry(suffix.to_string()).or_default().push(route),
                    None => exact.entry(host).or_default().push(route),
                },
            }
        }

        for routes in exact
            .values_mut()
            .chain(wildcard.values_mut())
            .chain([&mut any_host])
        {
            routes.sort_by_key(|r| route_priority(&r.path));
        }

        Self {
            exact_routes: exact,
            wildcard_routes: wildcard,
            any_host_routes: any_host,
            default_backend: value.default_backend.map(backend_host),
        }
    }
}

/// Longer paths first, and `Exact` before `Prefix` for the same length
fn route_priority(path: &HttpPath) -> (Reverse<usize>, bool) {
    match path {
        HttpPath::Exact(p) => (Reverse(p.len()), false),
        HttpPath::Prefix(p) => (Reverse(p.trim_end_matches('/').len()), true),
    }
}

trait IsMatch {
    fn is_match(&self, haystack: &str) -> bool;
}

impl IsMatch for HttpPath {
    fn is_match(&self, haystack: &str) -> bool {
        match self {
            // Prefix matching is done element-wise: `/foo` matches `/foo` and `/foo/bar`,
            // but not `/foobar`. A trailing slash in the prefix is ignored.
            HttpPath::Prefix(prefix) => {
                let prefix = prefix.trim_end_matches('/');
                haystack
                    .strip_prefix(prefix)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
            }
            HttpPath::Exact(exact) => haystack == exact,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProxyMap;
    use pingress_config::{Backend, HttpPath, PathRule, PingressConfiguration, Port};

    fn backend(name: &str, port: u16) -> Backend {
        Backend::Service {
            name: name.to_string(),
            namespace: "default".to_string(),
            port: Port::Number(port),
        }
    }

    fn rule(host: &str, path: HttpPath, name: &str, port: u16) -> PathRule {
        PathRule {
            host: Some(host.to_string()),
            tls: None,
            path,
            backend: backend(name, port),
        }
    }

    fn proxy_map(rules: Vec<PathRule>) -> ProxyMap {
        ProxyMap::from(PingressConfiguration {
            rules,
            default_backend: None,
        })
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
            Some("backend.default.svc:80")
        );
        assert_eq!(
            map.get_backend("test.example.com", "/api/v1/users"),
            Some("backend.default.svc:80")
        );
        assert_eq!(map.get_backend("test.example.com", "/"), None);
    }

    #[test]
    fn prefix_matches_by_path_element() {
        let map = proxy_map(vec![rule(
            "test.example.com",
            HttpPath::Prefix("/api/".to_string()),
            "backend",
            80,
        )]);

        assert!(map.get_backend("test.example.com", "/api").is_some());
        assert!(map.get_backend("test.example.com", "/api/").is_some());
        assert!(map.get_backend("test.example.com", "/api/v1").is_some());
        assert_eq!(map.get_backend("test.example.com", "/apiv2"), None);
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
            Some("backend-api.default.svc:8080")
        );
        assert_eq!(map.get_backend("test.example.com", "/v1/users/1"), None);
        assert_eq!(map.get_backend("test.example.com", "/v1"), None);
    }

    #[test]
    fn multiple_paths_for_same_host() {
        let map = proxy_map(vec![
            rule(
                "test.example.com",
                HttpPath::Prefix("/".to_string()),
                "web",
                80,
            ),
            rule(
                "test.example.com",
                HttpPath::Prefix("/api".to_string()),
                "api",
                80,
            ),
            rule(
                "test.example.com",
                HttpPath::Exact("/api".to_string()),
                "api-root",
                80,
            ),
        ]);

        assert_eq!(
            map.get_backend("test.example.com", "/index.html"),
            Some("web.default.svc:80")
        );
        assert_eq!(
            map.get_backend("test.example.com", "/api/users"),
            Some("api.default.svc:80")
        );
        assert_eq!(
            map.get_backend("test.example.com", "/api"),
            Some("api-root.default.svc:80")
        );
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
    fn host_is_case_insensitive() {
        let map = proxy_map(vec![rule(
            "Test.Example.com",
            HttpPath::Prefix("/".to_string()),
            "backend",
            80,
        )]);

        assert!(map.get_backend("TEST.example.COM", "/").is_some());
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
            Some("backend.default.svc:80")
        );
        assert_eq!(
            map.get_backend("bar.example.net", "/index.html"),
            Some("backend.default.svc:80")
        );
        assert_eq!(map.get_backend("example.net", "/"), None);
        assert_eq!(map.get_backend("foo.example.com", "/"), None);
    }

    #[test]
    fn wildcard_host_matches_single_label_only() {
        let map = proxy_map(vec![rule(
            "*.example.net",
            HttpPath::Prefix("/".to_string()),
            "backend",
            80,
        )]);

        assert_eq!(map.get_backend("a.b.example.net", "/"), None);
        assert_eq!(map.get_backend("a.example.net.evil.com", "/"), None);
        assert_eq!(map.get_backend("aXexampleYnet", "/"), None);
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
            Some("exact.default.svc:80")
        );
        assert_eq!(
            map.get_backend("other.example.com", "/"),
            Some("wildcard.default.svc:80")
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
            Some("exact.default.svc:80")
        );
        assert_eq!(
            map.get_backend("test.example.com", "/other"),
            Some("wildcard.default.svc:80")
        );
    }

    #[test]
    fn rule_without_host_and_default_backend() {
        let map = ProxyMap::from(PingressConfiguration {
            rules: vec![
                rule(
                    "test.example.com",
                    HttpPath::Prefix("/".to_string()),
                    "exact",
                    80,
                ),
                PathRule {
                    host: None,
                    tls: None,
                    path: HttpPath::Prefix("/any".to_string()),
                    backend: backend("any", 80),
                },
            ],
            default_backend: Some(backend("fallback", 8080)),
        });

        assert_eq!(
            map.get_backend("test.example.com", "/any"),
            Some("exact.default.svc:80")
        );
        assert_eq!(
            map.get_backend("other.example.com", "/any/thing"),
            Some("any.default.svc:80")
        );
        assert_eq!(
            map.get_backend("other.example.com", "/"),
            Some("fallback.default.svc:8080")
        );
    }

    #[test]
    fn empty_configuration() {
        let map = proxy_map(vec![]);

        assert_eq!(map.get_backend("test.example.com", "/"), None);
    }
}
