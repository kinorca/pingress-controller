use crate::http_proxy::PingressHttpProxy;
use crate::state::{ProxyState, SharedState};
use crate::watcher::run_reload;
use log::error;
use pingora::listeners::tls::TlsSettings;
use pingora::server::Server;
use pingora::services::ServiceWithDependents;
use pingora::tls::ext::{ssl_use_certificate, ssl_use_private_key};
use pingora::tls::ssl::{NameType, SslAcceptor, SslMethod, SslRef};
use std::error::Error;
use std::path::Path;
use std::thread::spawn;

mod http_proxy;
mod proxy_map;
mod state;
mod tls;
mod watcher;

pub struct ServerOptions {
    /// Path to the configuration file
    pub config_file: String,
    /// Directories to watch for reload. The directory of `config_file` is used when empty.
    pub watch: Vec<String>,
    pub listen_http: String,
    pub listen_https: String,
    pub listen_metrics: String,
}

pub fn run_server(options: ServerOptions) -> Result<(), Box<dyn Error>> {
    let state = SharedState::new(ProxyState::load(&options.config_file)?);

    let mut server = Server::new(None)?;
    server.bootstrap();

    let services: Vec<Box<dyn ServiceWithDependents>> = vec![create_http_proxy(
        &server,
        state.clone(),
        &options.listen_http,
        &options.listen_https,
    )?];

    let mut prometheus_service_http = pingora_prometheus::prometheus_http_service();
    prometheus_service_http.add_tcp(&options.listen_metrics);

    server.add_service(prometheus_service_http);
    server.add_services(services);

    let watch = if options.watch.is_empty() {
        let dir = Path::new(&options.config_file)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        vec![dir.to_string_lossy().into_owned()]
    } else {
        options.watch
    };
    let config_file = options.config_file;
    spawn(move || run_reload(&watch, &config_file, &state));

    server.run_forever();
}

fn create_http_proxy(
    server: &Server,
    state: SharedState,
    listen_http: &str,
    listen_https: &str,
) -> Result<Box<dyn ServiceWithDependents>, Box<dyn Error>> {
    let mut http_proxy = pingora::proxy::http_proxy_service(
        &server.configuration,
        PingressHttpProxy::new(state.clone()),
    );
    http_proxy.add_tcp(listen_http);

    http_proxy.add_tls_with_settings(listen_https, None, tls_settings(state)?);

    Ok(Box::new(http_proxy))
}

/// TLS settings selecting the certificate by SNI.
///
/// The certificate is set in the SNI callback instead of `TlsSettings::with_callbacks`, because
/// pingora resumes that handshake with a null certificate callback, which crashes the BoringSSL
/// bundled with `boring` 5.
fn tls_settings(state: SharedState) -> Result<TlsSettings, Box<dyn Error>> {
    let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls())?;
    builder.set_servername_callback(move |ssl, _alert| {
        use_certificate(ssl, &state);
        Ok(())
    });
    Ok(TlsSettings::from(builder))
}

/// Set the key pair for the SNI. Without one, the handshake fails for lack of a certificate.
fn use_certificate(ssl: &mut SslRef, state: &SharedState) {
    let Some(sni) = ssl.servername(NameType::HOST_NAME).map(str::to_string) else {
        return;
    };
    let state = state.get();
    let Some((pkey, cert)) = state.tls.get_tls(&sni) else {
        return;
    };

    if let Err(e) = ssl_use_certificate(ssl, cert) {
        error!("Error: Certificate for '{sni}': {e}");
        return;
    }
    if let Err(e) = ssl_use_private_key(ssl, pkey) {
        error!("Error: Private key for '{sni}': {e}");
    }
}
