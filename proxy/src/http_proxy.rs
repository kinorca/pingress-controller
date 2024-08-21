use crate::proxy_map::ProxyMap;
use async_trait::async_trait;
use pingora::prelude::{HttpPeer, ProxyHttp};
use pingora::ErrorType;
use pingress_config::PingressConfiguration;
use std::fs::File;

pub(crate) struct PingressHttpProxy {
    config_file: String,
}

impl PingressHttpProxy {
    pub(crate) fn new(config_file: String) -> Self {
        Self { config_file }
    }
}

pub struct Context {
    proxy_map: ProxyMap,
}

#[async_trait]
impl ProxyHttp for PingressHttpProxy {
    type CTX = Context;

    fn new_ctx(&self) -> Self::CTX {
        let config: PingressConfiguration = {
            let file = File::open(self.config_file.as_str()).unwrap();
            serde_json::from_reader(file).unwrap()
        };
        Context {
            proxy_map: ProxyMap::from(config),
        }
    }

    async fn upstream_peer(
        &self,
        session: &mut pingora::proxy::Session,
        ctx: &mut Self::CTX,
    ) -> pingora::Result<Box<HttpPeer>> {
        let host = match session
            .req_header()
            .headers
            .get("Host")
            .and_then(|h| h.to_str().ok())
        {
            Some(a) => a,
            None => return pingora::Error::err(ErrorType::InvalidHTTPHeader),
        };
        let path = session.req_header().uri.path();

        match ctx.proxy_map.get_backend(host, path) {
            None => pingora::Error::err(ErrorType::ConnectNoRoute),
            Some(backend_host) => Ok(Box::new(HttpPeer::new(
                backend_host,
                false,
                host.to_string(),
            ))),
        }
    }
}
