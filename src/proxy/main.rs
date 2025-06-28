use clap::Parser;
use log::{debug, info};
use proxy_server::run_server;

#[derive(Debug, Parser)]
struct Args {
    /// Listen host and port number
    #[clap(long, default_value = "0.0.0.0:80")]
    listen_http: String,

    /// Listen host and port number
    #[clap(long, default_value = "0.0.0.0:443")]
    listen_https: String,

    /// Path to a configuration file
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
    run_server(args.config, args.watch, args.listen_http, args.listen_https);
}
