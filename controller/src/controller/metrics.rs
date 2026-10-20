use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tracing::{error, info};

const CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
struct ResultLabels {
    result: &'static str,
}

impl ResultLabels {
    fn new(success: bool) -> Self {
        Self {
            result: if success { "success" } else { "error" },
        }
    }
}

pub struct Metrics {
    registry: Registry,
    reconciliations: Family<ResultLabels, Counter>,
    reconcile_duration: Histogram,
    syncs: Family<ResultLabels, Counter>,
    managed_ingresses: Gauge,
    tls_hosts: Gauge,
}

impl Metrics {
    pub fn new() -> Self {
        let mut registry = Registry::with_prefix("pingress_controller");

        let reconciliations = Family::<ResultLabels, Counter>::default();
        registry.register(
            "reconciliations",
            "Number of Ingress reconciliations",
            reconciliations.clone(),
        );
        // 10ms .. ~41s
        let reconcile_duration = Histogram::new(exponential_buckets(0.01, 2.0, 13));
        registry.register(
            "reconcile_duration_seconds",
            "Time taken by an Ingress reconciliation",
            reconcile_duration.clone(),
        );
        let syncs = Family::<ResultLabels, Counter>::default();
        registry.register(
            "syncs",
            "Number of regenerations of the managed resources",
            syncs.clone(),
        );
        let managed_ingresses = Gauge::default();
        registry.register(
            "managed_ingresses",
            "Number of Ingresses handled by this controller as of the last regeneration",
            managed_ingresses.clone(),
        );
        let tls_hosts = Gauge::default();
        registry.register(
            "tls_hosts",
            "Number of hosts with an available TLS key pair as of the last regeneration",
            tls_hosts.clone(),
        );

        Self {
            registry,
            reconciliations,
            reconcile_duration,
            syncs,
            managed_ingresses,
            tls_hosts,
        }
    }

    pub(crate) fn observe_reconcile(&self, success: bool, duration: Duration) {
        self.reconciliations
            .get_or_create(&ResultLabels::new(success))
            .inc();
        self.reconcile_duration.observe(duration.as_secs_f64());
    }

    pub(crate) fn sync_succeeded(&self, managed_ingresses: usize, tls_hosts: usize) {
        self.syncs.get_or_create(&ResultLabels::new(true)).inc();
        self.managed_ingresses.set(managed_ingresses as i64);
        self.tls_hosts.set(tls_hosts as i64);
    }

    pub(crate) fn sync_failed(&self) {
        self.syncs.get_or_create(&ResultLabels::new(false)).inc();
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Serve the metrics in the OpenMetrics text format at `/metrics`
pub async fn serve_metrics(listen: String, metrics: Arc<Metrics>) -> std::io::Result<()> {
    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(metrics);
    let listener = TcpListener::bind(&listen).await?;
    info!(listen, "Serving metrics");
    axum::serve(listener, app).await
}

async fn metrics_handler(State(metrics): State<Arc<Metrics>>) -> Response {
    let mut body = String::new();
    match encode(&mut body, &metrics.registry) {
        Ok(()) => ([(header::CONTENT_TYPE, CONTENT_TYPE)], body).into_response(),
        Err(e) => {
            error!(error = %e, "Cannot encode metrics");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
