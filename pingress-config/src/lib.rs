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
    use crate::PingressConfiguration;

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
}
