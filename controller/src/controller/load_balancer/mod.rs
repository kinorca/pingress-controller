//! LoadBalancer backend: every Ingress gets its own proxy (a Deployment) and a Service of type
//! LoadBalancer in its namespace, and the address of the Service is published to the Ingress
//! status.

mod reconcile;
mod resources;
mod status;

use crate::controller::load_balancer::reconcile::reconcile;
use crate::controller::{
    LogControllerResult, MANAGED_BY_LABEL, Metrics, handle_error, ingresses_referencing,
};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{ConfigMap, Secret, Service};
use k8s_openapi::api::networking::v1::Ingress;
use kube::runtime::Controller;
use kube::runtime::reflector::ObjectRef;
use kube::{Api, Client};
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::info;

const TYPE_LABEL: (&str, &str) = ("kinorca.com/pingress-controller-type", "load-balancer");
const NAME_LABEL: (&str, &str) = ("app.kubernetes.io/name", "pingress-proxy-server");
const INSTANCE_LABEL: &str = "app.kubernetes.io/instance";

/// Prefix of the names of the resources generated for an Ingress
const NAME_PREFIX: &str = "pingress-";
/// Service names are DNS-1035 labels
const MAX_NAME_LEN: usize = 63;

pub struct LoadBalancerOptions {
    pub image_pull_secret: Option<String>,
    pub proxy_server_image: String,
    /// Number of proxy pods per Ingress
    pub replicas: i32,
    /// `spec.loadBalancerClass` of the generated Services
    pub load_balancer_class: Option<String>,
}

pub async fn run_load_balancer<F>(
    client: Client,
    shutdown_signal: F,
    options: LoadBalancerOptions,
    metrics: Arc<Metrics>,
) where
    F: Future<Output = ()> + Send + Sync + 'static,
{
    info!(
        proxy_server_image = options.proxy_server_image,
        replicas = options.replicas,
        load_balancer_class = options.load_balancer_class,
        "Starting LoadBalancer controller"
    );

    let ingress_api: Api<Ingress> = Api::all(client.clone());
    let ingress_wc = kube::runtime::watcher::Config::default().any_semantic();

    let managed_wc = kube::runtime::watcher::Config::default().labels(&format!(
        "{}={},{}={}",
        MANAGED_BY_LABEL.0, MANAGED_BY_LABEL.1, TYPE_LABEL.0, TYPE_LABEL.1
    ));
    let service_api: Api<Service> = Api::all(client.clone());
    let deployment_api: Api<Deployment> = Api::all(client.clone());
    let config_map_api: Api<ConfigMap> = Api::all(client.clone());

    // TLS secrets referenced by Ingresses, to follow their creation and deletion
    let tls_secret_api: Api<Secret> = Api::all(client.clone());
    let tls_secret_wc = kube::runtime::watcher::Config::default().fields("type=kubernetes.io/tls");

    let controller = Controller::new(ingress_api, ingress_wc);
    let store = controller.store();

    controller
        .graceful_shutdown_on(shutdown_signal)
        // Service status carries the address of the load balancer
        .owns(service_api, managed_wc.clone())
        .owns(deployment_api, managed_wc.clone())
        .owns(config_map_api, managed_wc)
        .watches(tls_secret_api, tls_secret_wc, move |secret| {
            ingresses_referencing(&store, &secret)
        })
        .run(
            |i, c| async move {
                let start = Instant::now();
                let result = reconcile(i, c.clone()).await;
                c.metrics.observe_reconcile(result.is_ok(), start.elapsed());
                result
            },
            handle_error,
            Arc::new(Context::new(client, options, metrics)),
        )
        .log_controller_result()
        .await;

    info!("LoadBalancer controller stopped");
}

struct Context {
    client: Client,
    options: LoadBalancerOptions,
    metrics: Arc<Metrics>,
    /// Number of hosts with TLS of each managed Ingress, for the metrics
    tls_hosts: Mutex<HashMap<ObjectRef<Ingress>, usize>>,
}

impl Context {
    fn new(client: Client, options: LoadBalancerOptions, metrics: Arc<Metrics>) -> Self {
        Self {
            client,
            options,
            metrics,
            tls_hosts: Mutex::new(HashMap::new()),
        }
    }

    fn sync_succeeded(&self, ingress: ObjectRef<Ingress>, tls_hosts: Option<usize>) {
        let mut map = self.tls_hosts.lock().expect("lock is not poisoned");
        match tls_hosts {
            Some(n) => map.insert(ingress, n),
            None => map.remove(&ingress),
        };
        self.metrics
            .sync_succeeded(map.len(), map.values().sum::<usize>());
    }
}

/// Name of the Deployment, Service and ConfigMap generated for the Ingress.
///
/// Ingress names are DNS subdomains while Service names are DNS-1035 labels, so dots are replaced
/// and long names are truncated. A hash of the Ingress name is appended in those cases to keep
/// names unique.
fn resource_name(ingress: &str) -> String {
    let name = format!("{NAME_PREFIX}{}", ingress.replace('.', "-"));
    if name.len() <= MAX_NAME_LEN && !ingress.contains('.') {
        return name;
    }

    let hash = format!("{:08x}", fnv1a(ingress.as_bytes()));
    let head = &name[..name.len().min(MAX_NAME_LEN - hash.len() - 1)];
    format!("{}-{hash}", head.trim_end_matches('-'))
}

/// 32-bit FNV-1a, which is stable across builds unlike `DefaultHasher`
fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c9dc5, |hash, b| {
        (hash ^ u32::from(*b)).wrapping_mul(0x01000193)
    })
}

/// Labels of the generated resources
fn manifest_labels(name: &str) -> BTreeMap<String, String> {
    let mut labels = selector_labels(name);
    labels.insert(
        MANAGED_BY_LABEL.0.to_string(),
        MANAGED_BY_LABEL.1.to_string(),
    );
    labels.insert(TYPE_LABEL.0.to_string(), TYPE_LABEL.1.to_string());
    labels
}

/// Labels selecting the proxy pods of the Ingress
fn selector_labels(name: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (NAME_LABEL.0.to_string(), NAME_LABEL.1.to_string()),
        (INSTANCE_LABEL.to_string(), name.to_string()),
    ])
}

#[cfg(test)]
mod tests {
    use super::{MAX_NAME_LEN, resource_name};

    #[test]
    fn resource_name_of_short_name() {
        assert_eq!(resource_name("web"), "pingress-web");
    }

    #[test]
    fn resource_name_with_dots() {
        let name = resource_name("www.example.com");
        assert!(name.starts_with("pingress-www-example-com-"));
        assert_eq!(name.len(), "pingress-www-example-com-".len() + 8);
        // Not the same as the Ingress named with hyphens
        assert_ne!(name, resource_name("www-example-com"));
    }

    #[test]
    fn resource_name_of_long_name() {
        let long = "a".repeat(100);
        let name = resource_name(&long);
        assert_eq!(name.len(), MAX_NAME_LEN);
        assert_ne!(name, resource_name(&"a".repeat(101)));
        assert_eq!(name, resource_name(&long));
    }

    #[test]
    fn resource_name_does_not_end_with_hyphen_before_hash() {
        // Truncated right after a hyphen
        let ingress = format!("{}-{}", "a".repeat(44), "b".repeat(20));
        let name = resource_name(&ingress);
        assert!(!name.contains("--"));
        assert!(name.len() <= MAX_NAME_LEN);
    }
}
