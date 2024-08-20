mod controller;

use crate::controller::run_node_port;
use clap::{Parser, ValueEnum};
use kube::Client;
use log::info;

#[derive(Debug, Copy, Clone, ValueEnum)]
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
    #[clap(long, default_value = "pingora-system")]
    namespace: String,

    /// Node selector labels. (--backend=HostPort only) (e.g.: "example.com/node-type=external-network")
    #[clap(long, value_delimiter = ',', num_args = 0..)]
    node_selector: Vec<String>,
}

#[tokio::main]
async fn main() {
    println!("Hello, world!");

    let args = Args::parse();

    let client = Client::try_default().await.unwrap();

    match args.backend {
        Type::HostPort => {
            run_node_port(
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
            unimplemented!("--backend=LoadBalancer is not implemented")
        }
    }
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen signal");
    info!("Receive signal");
}
