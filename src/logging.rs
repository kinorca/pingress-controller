use tracing_subscriber::EnvFilter;

/// Install the JSON structured logger.
///
/// The level is configured by `RUST_LOG` (`info` when unset). Records of the `log` crate (e.g.
/// from pingora) are forwarded to `tracing` as well.
pub(crate) fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_current_span(true)
        .with_span_list(false)
        .with_target(true)
        .with_ansi(false)
        .with_env_filter(filter)
        .init();
}
