use clap::{Parser, ValueEnum};
use controller::{Metrics, run_host_port, serve_metrics};
use kube::Client;
use std::sync::Arc;
use tokio::signal::unix::SignalKind;
use tracing::{debug, error, info};

#[path = "../logging.rs"]
mod logging;

#[derive(Debug, Copy, Clone, ValueEnum)]
#[value(rename_all = "PascalCase")]
enum Type {
    HostPort,
    /// Not implemented yet
    LoadBalancer,
}

#[derive(Parser, Debug)]
struct Args {
    /// HTTP Backend
    #[clap(long)]
    backend: Type,

    /// Image pull secret to pull pingress-proxy-server
    #[clap(long)]
    image_pull_secret: Option<String>,

    #[clap(long, default_value = "ghcr.io/kinorca/pingress-proxy-server:latest")]
    proxy_server_image: String,

    /// Proxy deployment namespace (--backend=HostPort only)
    #[clap(long, default_value = "pingress-system")]
    namespace: String,

    /// Node selector labels. (--backend=HostPort only) (e.g.: "example.com/node-type=external-network")
    #[clap(long, value_delimiter = ',', num_args = 0.., value_parser = parse_key_value)]
    node_selector: Vec<(String, String)>,

    /// Listen host and port number for Prometheus metrics
    #[clap(long, default_value = "0.0.0.0:9090")]
    listen_metrics: String,
}

fn parse_key_value(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((k, v)) if !k.is_empty() => Ok((k.to_string(), v.to_string())),
        _ => Err(format!("expected KEY=VALUE: '{s}'")),
    }
}

#[tokio::main]
async fn main() {
    logging::init();

    let args = Args::parse();

    info!(backend = ?args.backend, "Starting pingress controller");
    debug!(?args, "Command line arguments");

    let client = match Client::try_default().await {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "Cannot create Kubernetes client");
            std::process::exit(1);
        }
    };

    let metrics = Arc::new(Metrics::new());
    tokio::spawn({
        let metrics = metrics.clone();
        let listen = args.listen_metrics.clone();
        async move {
            if let Err(e) = serve_metrics(listen.clone(), metrics).await {
                error!(listen, error = %e, "Cannot serve metrics");
            }
        }
    });

    match args.backend {
        Type::HostPort => {
            run_host_port(
                client,
                shutdown_signal(),
                args.namespace,
                args.node_selector.into_iter().collect(),
                args.image_pull_secret,
                args.proxy_server_image,
                metrics,
            )
            .await
        }
        Type::LoadBalancer => {
            error!(backend = ?args.backend, "--backend=LoadBalancer is not implemented yet");
            std::process::exit(2);
        }
    }
}

async fn shutdown_signal() {
    let mut terminate =
        tokio::signal::unix::signal(SignalKind::terminate()).expect("Failed to listen signal");
    let signal = tokio::select! {
        _ = terminate.recv() => "SIGTERM",
        r = tokio::signal::ctrl_c() => {
            r.expect("Failed to listen signal");
            "SIGINT"
        }
    };
    info!(signal, "Received shutdown signal");
}
