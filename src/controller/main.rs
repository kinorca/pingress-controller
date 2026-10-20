use clap::{Parser, ValueEnum};
use controller::run_host_port;
use kube::Client;
use log::{debug, error, info};
use tokio::signal::unix::SignalKind;

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
}

fn parse_key_value(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((k, v)) if !k.is_empty() => Ok((k.to_string(), v.to_string())),
        _ => Err(format!("expected KEY=VALUE: '{s}'")),
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    info!("Starting pingress controller");

    let args = Args::parse();
    debug!("Command line arguments: {args:?}");

    let client = match Client::try_default().await {
        Ok(c) => c,
        Err(e) => {
            error!("Cannot create Kubernetes client: {e}");
            std::process::exit(1);
        }
    };

    match args.backend {
        Type::HostPort => {
            run_host_port(
                client,
                shutdown_signal(),
                args.namespace,
                args.node_selector.into_iter().collect(),
                args.image_pull_secret,
                args.proxy_server_image,
            )
            .await
        }
        Type::LoadBalancer => {
            error!("--backend=LoadBalancer is not implemented yet");
            std::process::exit(2);
        }
    }
}

async fn shutdown_signal() {
    let mut terminate =
        tokio::signal::unix::signal(SignalKind::terminate()).expect("Failed to listen signal");
    tokio::select! {
        _ = terminate.recv() => {}
        r = tokio::signal::ctrl_c() => r.expect("Failed to listen signal"),
    }
    info!("Receive signal");
}
