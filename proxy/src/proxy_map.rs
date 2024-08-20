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
