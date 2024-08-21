use crate::http_proxy::PingressHttpProxy;
use crate::tls::{GetTls, TlsMap};
use crate::watcher::run_reload;
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
use std::sync::{Arc, RwLock};
use std::thread::spawn;

mod http_proxy;
mod proxy_map;
mod tls;
mod watcher;

#[derive(Debug, Parser)]
struct Args {
    /// Listen host and port number
    #[clap(long, default_value = "0.0.0.0:80")]
    listen_http: String,

    /// Listen host and port number
    #[clap(long, default_value = "0.0.0.0:443")]
    listen_https: String,

    /// Path to configuration file
    #[clap(long)]
    config: String,

    /// Watch directory
    #[clap(long)]
    watch: String,
}

fn main() {
    env_logger::init();

    info!("Starting pingress proxy server");

    let args = Args::parse();

    debug!("Command line args: {args:?}");

    let mut server = Server::new(None).unwrap();
    server.bootstrap();

    let tls = {
        let config: PingressConfiguration = {
            let file = File::open(args.config.as_str()).unwrap();
            serde_json::from_reader(file).unwrap()
        };
        Arc::new(RwLock::new(TlsMap::from(config)))
    };

    let services: Vec<Box<dyn Service>> = { vec![create_http_proxy(&server, &args, tls.clone())] };

    let mut prometheus_service_http =
        pingora::services::listening::Service::prometheus_http_service();
    prometheus_service_http.add_tcp("127.0.0.1:9090");

    server.add_service(prometheus_service_http);
    server.add_services(services);

    spawn(move || {
        run_reload(args.watch.as_str(), args.config.as_str(), &tls);
    });

    server.run_forever();
}

fn create_http_proxy(server: &Server, args: &Args, tls: Arc<RwLock<TlsMap>>) -> Box<dyn Service> {
    let mut http_proxy = pingora::proxy::http_proxy_service(
        &server.configuration,
        PingressHttpProxy::new(args.config.clone()),
    );
    http_proxy.add_tcp(args.listen_http.as_str());

    http_proxy.add_tls_with_settings(
        args.listen_https.as_str(),
        None,
        TlsSettings::with_callbacks(TlsAcceptor::new(tls).into()).unwrap(),
    );

    Box::new(http_proxy)
}

struct TlsAcceptor {
    tls: Arc<RwLock<TlsMap>>,
}

impl TlsAcceptor {
    fn new(tls: Arc<RwLock<TlsMap>>) -> Self {
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
        let keys = match self.tls.read() {
            Ok(tls) => ssl
                .servername(NameType::HOST_NAME)
                .and_then(|sni| tls.get_tls(sni)),
            Err(err) => {
                error!("Error: Cannot lock tls map: {err}");
                return;
            }
        };

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
