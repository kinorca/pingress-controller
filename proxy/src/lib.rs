use crate::http_proxy::PingressHttpProxy;
use crate::tls::GetTls;
use crate::tls::TlsMap;
use crate::watcher::run_reload;
use async_trait::async_trait;
use log::error;
use pingora::listeners::tls::TlsSettings;
use pingora::listeners::{TlsAccept, TlsAcceptCallbacks};
use pingora::protocols::tls::TlsRef;
use pingora::server::Server;
use pingora::services::Service;
use pingora::tls::ext::{ssl_use_certificate, ssl_use_private_key};
use pingora::tls::ssl::NameType;
use pingress_config::PingressConfiguration;
use std::fs::File;
use std::sync::Arc;
use std::thread::spawn;
use tokio::sync::RwLock;

mod http_proxy;
mod proxy_map;
mod tls;
mod watcher;

pub fn run_server(
    config_file: String,
    watch_directory: String,
    listen_http: String,
    listen_https: String,
) {
    let mut server = Server::new(None).unwrap();
    server.bootstrap();

    let tls = {
        let config: PingressConfiguration = {
            let file = File::open(&config_file).unwrap();
            serde_json::from_reader(file).unwrap()
        };
        Arc::new(RwLock::new(TlsMap::from(config)))
    };
    let services: Vec<Box<dyn Service>> = {
        vec![create_http_proxy(
            &server,
            config_file.clone(),
            &listen_http,
            &listen_https,
            tls.clone(),
        )]
    };

    let mut prometheus_service_http =
        pingora::services::listening::Service::prometheus_http_service();
    prometheus_service_http.add_tcp("127.0.0.1:9090");

    server.add_service(prometheus_service_http);
    server.add_services(services);

    spawn(move || {
        run_reload(&watch_directory, &config_file, &tls);
    });

    server.run_forever();
}

fn create_http_proxy(
    server: &Server,
    config_file: String,
    listen_http: &str,
    listen_https: &str,
    tls: Arc<RwLock<TlsMap>>,
) -> Box<dyn Service> {
    let mut http_proxy = pingora::proxy::http_proxy_service(
        &server.configuration,
        PingressHttpProxy::new(config_file),
    );
    http_proxy.add_tcp(listen_http);

    http_proxy.add_tls_with_settings(
        listen_https,
        None,
        TlsSettings::with_callbacks(TlsAcceptor::new(tls).into()).unwrap(),
    );

    Box::new(http_proxy)
}

pub struct TlsAcceptor {
    tls: Arc<RwLock<TlsMap>>,
}

impl TlsAcceptor {
    pub fn new(tls: Arc<RwLock<TlsMap>>) -> Self {
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
    async fn certificate_callback(&self, ssl: &mut TlsRef) -> () {
        let tls = self.tls.read().await;
        let keys = ssl
            .servername(NameType::HOST_NAME)
            .and_then(|sni| tls.get_tls(sni));

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
