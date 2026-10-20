use crate::state::{ProxyState, SharedState};
use async_trait::async_trait;
use pingora::ErrorType;
use pingora::http::{RequestHeader, ResponseHeader};
use pingora::prelude::{HttpPeer, ProxyHttp};
use pingora::proxy::Session;
use std::sync::Arc;

pub(crate) struct PingressHttpProxy {
    state: SharedState,
}

impl PingressHttpProxy {
    pub(crate) fn new(state: SharedState) -> Self {
        Self { state }
    }
}

pub struct Context {
    /// Snapshot of the state, so that one request is handled with one configuration
    state: Arc<ProxyState>,
}

#[async_trait]
impl ProxyHttp for PingressHttpProxy {
    type CTX = Context;

    fn new_ctx(&self) -> Self::CTX {
        Context {
            state: self.state.get(),
        }
    }

    /// Redirect plain HTTP requests to HTTPS for hosts that have a certificate
    async fn request_filter(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> pingora::Result<bool> {
        let is_tls = session.digest().is_some_and(|d| d.ssl_digest.is_some());
        if is_tls {
            return Ok(false);
        }

        let Some(host) = request_host(session.req_header()) else {
            return Ok(false);
        };
        if !ctx.state.tls.contains(host) {
            return Ok(false);
        }

        let path_and_query = session
            .req_header()
            .uri
            .path_and_query()
            .map_or("/", |p| p.as_str());
        let location = format!("https://{host}{path_and_query}");

        let mut resp = ResponseHeader::build(308, Some(2))?;
        resp.insert_header("Location", location)?;
        resp.insert_header("Content-Length", "0")?;
        session.write_response_header(Box::new(resp), true).await?;
        Ok(true)
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> pingora::Result<Box<HttpPeer>> {
        let Some(host) = request_host(session.req_header()) else {
            return pingora::Error::e_explain(ErrorType::HTTPStatus(400), "no host in request");
        };
        let path = session.req_header().uri.path();

        let Some(backend) = ctx.state.proxy_map.get_backend(host, path) else {
            return pingora::Error::e_explain(
                ErrorType::HTTPStatus(404),
                format!("no route for {host}{path}"),
            );
        };

        // Resolve asynchronously; `HttpPeer::new` resolves with a blocking call and panics on
        // failure.
        let addr = tokio::net::lookup_host(backend)
            .await
            .ok()
            .and_then(|mut addrs| addrs.next());
        match addr {
            Some(addr) => Ok(Box::new(HttpPeer::new(addr, false, String::new()))),
            None => pingora::Error::e_explain(
                ErrorType::HTTPStatus(502),
                format!("cannot resolve backend {backend}"),
            ),
        }
    }
}

/// Host name of the request without port, from the `Host` header or the URI authority (HTTP/2)
fn request_host(req: &RequestHeader) -> Option<&str> {
    let authority = req
        .headers
        .get("Host")
        .and_then(|h| h.to_str().ok())
        .or_else(|| req.uri.authority().map(|a| a.as_str()))?;
    let host = strip_port(authority);
    (!host.is_empty()).then_some(host)
}

fn strip_port(authority: &str) -> &str {
    if authority.starts_with('[') {
        // IPv6 literal: `[::1]:8080`
        return match authority.find(']') {
            Some(end) => &authority[..=end],
            None => authority,
        };
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => authority,
    }
}

#[cfg(test)]
mod tests {
    use super::{request_host, strip_port};
    use pingora::http::RequestHeader;

    #[test]
    fn strip_port_from_authority() {
        assert_eq!(strip_port("example.com"), "example.com");
        assert_eq!(strip_port("example.com:8080"), "example.com");
        assert_eq!(strip_port("[::1]:8080"), "[::1]");
        assert_eq!(strip_port("[::1]"), "[::1]");
    }

    #[test]
    fn host_from_header() {
        let mut req = RequestHeader::build("GET", b"/", None).unwrap();
        req.insert_header("Host", "example.com:80").unwrap();

        assert_eq!(request_host(&req), Some("example.com"));
    }

    #[test]
    fn host_from_uri_authority() {
        let mut req = RequestHeader::build("GET", b"/path", None).unwrap();
        req.set_uri("https://example.com:443/path".parse().unwrap());

        assert_eq!(request_host(&req), Some("example.com"));
    }

    #[test]
    fn no_host() {
        let req = RequestHeader::build("GET", b"/", None).unwrap();

        assert_eq!(request_host(&req), None);
    }
}
