mod config_map;
mod daemonset;
mod reconcile;
mod secrets;

use crate::controller::host_port::reconcile::reconcile;
use crate::controller::{
    LogControllerResult, MANAGED_BY_LABEL, Metrics, handle_error, ingresses_referencing, is_managed,
};
use k8s_openapi::api::apps::v1::DaemonSet;
use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use k8s_openapi::api::networking::v1::Ingress;
use kube::runtime::Controller;
use kube::runtime::reflector::{ObjectRef, Store};
use kube::{Api, Client};
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::info;

const TLS_SECRET_NAME: &str = "pingress-tls-secret";
const CONFIG_MAP_NAME: &str = "pingress-config";

pub async fn run_host_port<F>(
    client: Client,
    shutdown_signal: F,
    namespace: String,
    node_selector: BTreeMap<String, String>,
    image_pull_secret: Option<String>,
    proxy_server_image: String,
    metrics: Arc<Metrics>,
) where
    F: Future<Output = ()> + Send + Sync + 'static,
{
    info!(
        namespace,
        ?node_selector,
        proxy_server_image,
        "Starting HostPort controller"
    );

    let ingress_api: Api<Ingress> = Api::all(client.clone());
    let ingress_wc = kube::runtime::watcher::Config::default().any_semantic();

    let managed_wc = kube::runtime::watcher::Config::default()
        .labels(&format!("{}={}", MANAGED_BY_LABEL.0, MANAGED_BY_LABEL.1));
    let daemonset_api: Api<DaemonSet> = Api::namespaced(client.clone(), namespace.as_str());
    let config_map_api: Api<ConfigMap> = Api::namespaced(client.clone(), namespace.as_str());
    let managed_secret_api: Api<Secret> = Api::namespaced(client.clone(), namespace.as_str());

    // TLS secrets referenced by Ingresses, to follow certificate renewal
    let tls_secret_api: Api<Secret> = Api::all(client.clone());
    let tls_secret_wc = kube::runtime::watcher::Config::default().fields("type=kubernetes.io/tls");

    let controller = Controller::new(ingress_api, ingress_wc);
    let store = controller.store();

    controller
        .graceful_shutdown_on(shutdown_signal)
        .watches(daemonset_api, managed_wc.clone(), {
            let store = store.clone();
            move |_| any_managed_ingress(&store)
        })
        .watches(config_map_api, managed_wc.clone(), {
            let store = store.clone();
            move |_| any_managed_ingress(&store)
        })
        .watches(managed_secret_api, managed_wc, {
            let store = store.clone();
            move |_| any_managed_ingress(&store)
        })
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
            Arc::new(Context::new(
                client,
                namespace,
                node_selector,
                image_pull_secret,
                proxy_server_image,
                metrics,
            )),
        )
        .log_controller_result()
        .await;

    info!("HostPort controller stopped");
}

/// Any one managed Ingress; reconciling any of them regenerates all managed resources
fn any_managed_ingress(store: &Store<Ingress>) -> Option<ObjectRef<Ingress>> {
    store
        .state()
        .iter()
        .find(|i| is_managed(i))
        .map(|i| ObjectRef::from_obj(i.as_ref()))
}

struct Context {
    client: Client,
    namespace: String,
    node_selector: BTreeMap<String, String>,
    image_pull_secret: Option<String>,
    proxy_server_image: String,
    metrics: Arc<Metrics>,
    /// Serializes generation of managed resources so that a stale Ingress list never overwrites a
    /// newer one
    sync_lock: Mutex<()>,
}

impl Context {
    fn new(
        client: Client,
        namespace: String,
        node_selector: BTreeMap<String, String>,
        image_pull_secret: Option<String>,
        proxy_server_image: String,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            client,
            namespace,
            node_selector,
            image_pull_secret,
            proxy_server_image,
            metrics,
            sync_lock: Mutex::new(()),
        }
    }
}

fn manifest_labels() -> Option<BTreeMap<String, String>> {
    Some(BTreeMap::from([
        (
            MANAGED_BY_LABEL.0.to_string(),
            MANAGED_BY_LABEL.1.to_string(),
        ),
        (
            "kinorca.com/pingress-controller-type".to_string(),
            "host-port".to_string(),
        ),
    ]))
}
