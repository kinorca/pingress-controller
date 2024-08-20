use crate::proxy_map::ProxyMap;
use async_trait::async_trait;
use pingora::prelude::{HttpPeer, ProxyHttp};
use pingora::ErrorType;

pub(crate) struct PingressHttpProxy {
    proxy_map: ProxyMap,
}

impl PingressHttpProxy {
    pub(crate) fn new(proxy_map: ProxyMap) -> Self {
        Self { proxy_map }
    }
}

#[async_trait]
impl ProxyHttp for PingressHttpProxy {
    type CTX = ();

    fn new_ctx(&self) -> Self::CTX {}

    async fn upstream_peer(
        &self,
        session: &mut pingora::proxy::Session,
        _ctx: &mut Self::CTX,
    ) -> pingora::Result<Box<HttpPeer>> {
        let authority = match session.req_header().uri.authority() {
            Some(a) => a,
            None => return pingora::Error::err(ErrorType::InvalidHTTPHeader),
        };
        let path = session.req_header().uri.path();

        match self.proxy_map.get_backend(authority.host(), path) {
            None => pingora::Error::err(ErrorType::ConnectNoRoute),
            Some(backend_host) => Ok(Box::new(HttpPeer::new(
                backend_host,
                false,
                authority.to_string(),
            ))),
        }
    }
}
