use async_trait::async_trait;
use http::Response;
use pingora::apps::http_app::{HttpServer, ServeHttp};
use pingora::protocols::http::ServerSession;
use pingora::services::listening::Service;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::error;

const CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct RequestLabels {
    pub(crate) method: String,
    pub(crate) status: u16,
    /// Backend address, or empty when no route matched. The request host is not used because
    /// clients can send arbitrary values, which would make the cardinality unbounded.
    pub(crate) backend: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct BackendLabels {
    pub(crate) backend: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub(crate) struct ResultLabels {
    pub(crate) result: &'static str,
}

pub(crate) struct Metrics {
    registry: Registry,
    requests: Family<RequestLabels, Counter>,
    request_duration: Family<BackendLabels, Histogram>,
    upstream_resolve_errors: Family<BackendLabels, Counter>,
    https_redirects: Counter,
    tls_certificate_not_found: Counter,
    config_reloads: Family<ResultLabels, Counter>,
    config_last_success: Gauge<f64, AtomicU64>,
    routes: Gauge,
    tls_certificates: Gauge,
}

impl Metrics {
    pub(crate) fn new() -> Self {
        let mut registry = Registry::with_prefix("pingress_proxy");

        let requests = Family::<RequestLabels, Counter>::default();
        registry.register(
            "http_requests",
            "Number of HTTP requests handled",
            requests.clone(),
        );
        let request_duration =
            Family::<BackendLabels, Histogram>::new_with_constructor(duration_histogram as _);
        registry.register(
            "http_request_duration_seconds",
            "Time from receiving an HTTP request to finishing its response",
            request_duration.clone(),
        );
        let upstream_resolve_errors = Family::<BackendLabels, Counter>::default();
        registry.register(
            "upstream_resolve_errors",
            "Number of failures to resolve a backend address",
            upstream_resolve_errors.clone(),
        );
        let https_redirects = Counter::default();
        registry.register(
            "https_redirects",
            "Number of plain HTTP requests redirected to HTTPS",
            https_redirects.clone(),
        );
        let tls_certificate_not_found = Counter::default();
        registry.register(
            "tls_certificate_not_found",
            "Number of TLS handshakes without a certificate for the SNI",
            tls_certificate_not_found.clone(),
        );
        let config_reloads = Family::<ResultLabels, Counter>::default();
        registry.register(
            "config_reloads",
            "Number of configuration reloads",
            config_reloads.clone(),
        );
        let config_last_success = Gauge::<f64, AtomicU64>::default();
        registry.register(
            "config_last_success_timestamp_seconds",
            "Unix time when the configuration was last loaded successfully",
            config_last_success.clone(),
        );
        let routes = Gauge::default();
        registry.register(
            "routes",
            "Number of routes in the current configuration",
            routes.clone(),
        );
        let tls_certificates = Gauge::default();
        registry.register(
            "tls_certificates",
            "Number of TLS certificates in the current configuration",
            tls_certificates.clone(),
        );

        Self {
            registry,
            requests,
            request_duration,
            upstream_resolve_errors,
            https_redirects,
            tls_certificate_not_found,
            config_reloads,
            config_last_success,
            routes,
            tls_certificates,
        }
    }

    pub(crate) fn observe_request(&self, labels: RequestLabels, duration: Duration) {
        self.request_duration
            .get_or_create(&BackendLabels {
                backend: labels.backend.clone(),
            })
            .observe(duration.as_secs_f64());
        self.requests.get_or_create(&labels).inc();
    }

    pub(crate) fn inc_upstream_resolve_errors(&self, backend: &str) {
        self.upstream_resolve_errors
            .get_or_create(&BackendLabels {
                backend: backend.to_string(),
            })
            .inc();
    }

    pub(crate) fn inc_https_redirects(&self) {
        self.https_redirects.inc();
    }

    pub(crate) fn inc_tls_certificate_not_found(&self) {
        self.tls_certificate_not_found.inc();
    }

    /// Record a successful load of the configuration
    pub(crate) fn config_loaded(&self, routes: usize, tls_certificates: usize) {
        self.config_reloads
            .get_or_create(&ResultLabels { result: "success" })
            .inc();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        self.config_last_success.set(now.as_secs_f64());
        self.routes.set(routes as i64);
        self.tls_certificates.set(tls_certificates as i64);
    }

    pub(crate) fn config_load_failed(&self) {
        self.config_reloads
            .get_or_create(&ResultLabels { result: "error" })
            .inc();
    }
}

fn duration_histogram() -> Histogram {
    // 1ms .. ~16s
    Histogram::new(exponential_buckets(0.001, 2.0, 15))
}

/// HTTP application serving the metrics in the OpenMetrics text format
pub(crate) struct MetricsHttpApp {
    metrics: Arc<Metrics>,
}

#[async_trait]
impl ServeHttp for MetricsHttpApp {
    async fn response(&self, _http_session: &mut ServerSession) -> Response<Vec<u8>> {
        let mut body = String::new();
        if let Err(e) = encode(&mut body, &self.metrics.registry) {
            error!(error = %e, "Cannot encode metrics");
            return Response::builder()
                .status(500)
                .body(Vec::new())
                .expect("valid response");
        }
        Response::builder()
            .status(200)
            .header(http::header::CONTENT_TYPE, CONTENT_TYPE)
            .header(http::header::CONTENT_LENGTH, body.len())
            .body(body.into_bytes())
            .expect("valid response")
    }
}

pub(crate) fn metrics_http_service(metrics: Arc<Metrics>) -> Service<HttpServer<MetricsHttpApp>> {
    Service::new(
        "Prometheus metrics HTTP".to_string(),
        HttpServer::new_app(MetricsHttpApp { metrics }),
    )
}
