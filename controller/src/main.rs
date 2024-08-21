mod controller;

use crate::controller::{run_host_port, run_load_balancer};
use clap::{Parser, ValueEnum};
use kube::Client;
use log::{debug, info};
use tokio::signal::unix::SignalKind;

#[macro_export]
macro_rules! try_with_log {
    ($trying:expr) => {
        $trying.inspect_err(|e| ::log::error!("Error: {e}"))?
    };
}

#[derive(Debug, Copy, Clone, ValueEnum)]
#[value(rename_all = "PascalCase")]
enum Type {
    HostPort,
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
    #[clap(long, value_delimiter = ',', num_args = 0..)]
    node_selector: Vec<String>,
}

#[tokio::main]
async fn main() {
    env_logger::init();

    info!("Starting pingress controller");

    let args = Args::parse();
    debug!("Command line arguments: {args:?}");

    let client = Client::try_default().await.unwrap();

    match args.backend {
        Type::HostPort => {
            run_host_port(
                client,
                shutdown_signal(),
                args.namespace,
                args.node_selector,
                args.image_pull_secret,
                args.proxy_server_image,
            )
            .await
        }
        Type::LoadBalancer => {
            unimplemented!();
            run_load_balancer(
                client,
                shutdown_signal(),
                args.image_pull_secret,
                args.proxy_server_image,
            )
            .await
        }
    }
}

async fn shutdown_signal() {
    tokio::signal::unix::signal(SignalKind::terminate())
        .unwrap()
        .recv()
        .await
        .expect("Failed to listen signal");
    info!("Receive signal");
}
