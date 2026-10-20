use clap::Parser;
use proxy_server::{ServerOptions, run_server};
use tracing::{debug, error, info};

#[path = "../logging.rs"]
mod logging;

#[derive(Debug, Parser)]
struct Args {
    /// Listen host and port number
    #[clap(long, default_value = "0.0.0.0:80")]
    listen_http: String,

    /// Listen host and port number
    #[clap(long, default_value = "0.0.0.0:443")]
    listen_https: String,

    /// Listen host and port number for Prometheus metrics
    #[clap(long, default_value = "0.0.0.0:9090")]
    listen_metrics: String,

    /// Path to a configuration file
    #[clap(long)]
    config: String,

    /// Directories to watch for reloading (repeatable). Defaults to the directory of --config.
    #[clap(long)]
    watch: Vec<String>,
}

fn main() {
    logging::init();

    let args = Args::parse();

    info!("Starting pingress proxy server");
    debug!(?args, "Command line arguments");
    let result = run_server(ServerOptions {
        config_file: args.config,
        watch: args.watch,
        listen_http: args.listen_http,
        listen_https: args.listen_https,
        listen_metrics: args.listen_metrics,
    });
    if let Err(e) = result {
        error!(error = %e, "Cannot start pingress proxy server");
        std::process::exit(1);
    }
}
