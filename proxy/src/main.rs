use crate::http_proxy::PingressHttpProxy;
use crate::proxy_map::ProxyMap;
use crate::tls::{GetTls, TlsMap};
use async_trait::async_trait;
use clap::Parser;
use log::{debug, error, info};
use pingora::listeners::{TlsAccept, TlsSettings};
use pingora::protocols::ssl::server::TlsAcceptCallbacks;
use pingora::server::Server;
use pingora::services::Service;
use pingora::tls::ext::{ssl_use_certificate, ssl_use_private_key};
use pingora::tls::ssl::{NameType, SslRef};
use pingress_config::PingressConfiguration;
use std::fs::File;

mod http_proxy;
mod proxy_map;
mod tls;

#[derive(Debug, Parser)]
struct Args {
    /// Listen host and port number
    #[clap(long, default_value = "80")]
    listen_http: String,

    /// Listen host and port number
    #[clap(long, default_value = "443")]
    listen_https: String,

    /// Path to configuration file
    config: String,
}

fn main() {
    env_logger::init();

    info!("Starting pingress proxy server");

    let args = Args::parse();

    debug!("Command line args: {args:?}");

    let config: PingressConfiguration = {
        let file = File::open(args.config).unwrap();
        serde_json::from_reader(file).unwrap()
    };

    let mut server = Server::new(None).unwrap();
    server.bootstrap();

    let services: Vec<Box<dyn Service>> = vec![create_http_proxy(
        &server,
        &config,
        args.listen_http.as_str(),
        args.listen_https.as_str(),
    )];

    let mut prometheus_service_http =
        pingora::services::listening::Service::prometheus_http_service();
    prometheus_service_http.add_tcp("127.0.0.1:9090");

    server.add_service(prometheus_service_http);
    server.add_services(services);

    server.run_forever();
}

fn create_http_proxy(
    server: &Server,
    config: &PingressConfiguration,
    listen_http: &str,
    listen_https: &str,
) -> Box<dyn Service> {
    let mut http_proxy = pingora::proxy::http_proxy_service(
        &server.configuration,
        PingressHttpProxy::new(ProxyMap::from(config.clone())),
    );
    http_proxy.add_tcp(listen_http);

    http_proxy.add_tls_with_settings(
        listen_https,
        None,
        TlsSettings::with_callbacks(TlsAcceptor::new(TlsMap::from(config.clone())).into()).unwrap(),
    );

    Box::new(http_proxy)
}

struct TlsAcceptor {
    tls: TlsMap,
}

impl TlsAcceptor {
    fn new(tls: TlsMap) -> Self {
        Self { tls }
    }
}

impl From<TlsAcceptor> for TlsAcceptCallbacks {
    fn from(value: TlsAcceptor) -> Self {
        Box::new(value)
    }
}

#[async_trait]
impl TlsAccept for TlsAcceptor {
    async fn certificate_callback(&self, ssl: &mut SslRef) -> () {
        let keys = ssl
            .servername(NameType::HOST_NAME)
            .and_then(|sni| self.tls.get_tls(sni));

        if let Some((sni, pkey, cert)) = keys {
            if let Err(e) = ssl_use_certificate(ssl, &cert) {
                error!("Error: Certificate for '{sni}': {e}");
                return;
            }
            if let Err(e) = ssl_use_private_key(ssl, &pkey) {
                error!("Error: Private key for '{sni}': {e}");
                return;
            }
        }
    }
}
