use clap::Parser;
use log::{debug, error, info};
use proxy_server::{ServerOptions, run_server};

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
    env_logger::init();

    info!("Starting pingress proxy server");

    let args = Args::parse();

    debug!("Command line args: {args:?}");
    let result = run_server(ServerOptions {
        config_file: args.config,
        watch: args.watch,
        listen_http: args.listen_http,
        listen_https: args.listen_https,
        listen_metrics: args.listen_metrics,
    });
    if let Err(e) = result {
        error!("Cannot start pingress proxy server: {e}");
        std::process::exit(1);
    }
}
