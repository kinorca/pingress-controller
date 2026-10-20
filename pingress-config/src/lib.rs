use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PingressConfiguration {
    pub rules: Vec<PathRule>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PathRule {
    pub host: String,
    pub tls: Option<Tls>,
    pub path: HttpPath,
    pub backend: Backend,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tls {
    pub key: String,
    pub cert: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", content = "path")]
pub enum HttpPath {
    Prefix(String),
    Exact(String),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum Backend {
    Service {
        name: String,
        namespace: String,
        port: Port,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Port {
    Number(u16),
}

#[cfg(test)]
mod tests {
    use crate::{Backend, HttpPath, PathRule, PingressConfiguration, Port, Tls};
    use serde_json::json;

    #[test]
    fn can_parse() {
        let json = r#"
        {
            "rules": [
                {
                    "host": "test.example.com",
                    "path": {
                        "type": "Prefix",
                        "path": "/"
                    },
                    "backend": {
                        "type": "Service",
                        "name": "backend",
                        "namespace": "default",
                        "port": 80
                    }
                },
                {
                    "host": "*.example.net",
                    "path": {
                        "type": "Exact",
                        "path": "/v1/users"
                    },
                    "backend": {
                        "type": "Service",
                        "name": "backend-api",
                        "namespace": "default",
                        "port": 8080
                    }
                }
            ]
        }
        "#;

        serde_json::from_str::<PingressConfiguration>(json).expect("Can parse");
    }

    #[test]
    fn can_parse_tls() {
        let json = json!({
            "host": "secure.example.com",
            "tls": { "key": "/keys/tls.key", "cert": "/keys/tls.cert" },
            "path": { "type": "Prefix", "path": "/" },
            "backend": { "type": "Service", "name": "backend", "namespace": "app", "port": 443 }
        });

        let rule = serde_json::from_value::<PathRule>(json).expect("Can parse");
        let tls = rule.tls.expect("tls is set");
        assert_eq!(tls.key, "/keys/tls.key");
        assert_eq!(tls.cert, "/keys/tls.cert");
    }

    #[test]
    fn serialize_format() {
        let config = PingressConfiguration {
            rules: vec![PathRule {
                host: "test.example.com".to_string(),
                tls: Some(Tls {
                    key: "key".to_string(),
                    cert: "cert".to_string(),
                }),
                path: HttpPath::Exact("/v1".to_string()),
                backend: Backend::Service {
                    name: "backend".to_string(),
                    namespace: "default".to_string(),
                    port: Port::Number(8080),
                },
            }],
        };

        assert_eq!(
            serde_json::to_value(&config).unwrap(),
            json!({
                "rules": [{
                    "host": "test.example.com",
                    "tls": { "key": "key", "cert": "cert" },
                    "path": { "type": "Exact", "path": "/v1" },
                    "backend": {
                        "type": "Service",
                        "name": "backend",
                        "namespace": "default",
                        "port": 8080
                    }
                }]
            })
        );
    }

    #[test]
    fn reject_unknown_path_type() {
        let json = json!({ "type": "Regex", "path": "/.*" });

        assert!(serde_json::from_value::<HttpPath>(json).is_err());
    }
}
