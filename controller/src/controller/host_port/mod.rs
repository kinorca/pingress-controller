mod config_map;
mod daemonset;
mod ingresses;
mod reconcile;
mod secrets;

use crate::controller::host_port::reconcile::reconcile;
use crate::controller::{FINALIZER, LogControllerResult, handle_error};
use k8s_openapi::api::apps::v1::DaemonSet;
use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use k8s_openapi::api::networking::v1::Ingress;
use kube::runtime::Controller;
use kube::runtime::reflector::{ObjectRef, Store};
use kube::{Api, Client, ResourceExt};
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::Mutex;

const FIELD_MANAGER: &str = "kinorca.com/pingress-controller";
const TLS_SECRET_NAME: &str = "pingress-tls-secret";
const CONFIG_MAP_NAME: &str = "pingress-config";
const CONFIG_KEY: &str = "proxy.json";
const CONFIG_BASE_PATH: &str = "/etc/pingress/config";
const SECRET_BASE_PATH: &str = "/etc/pingress/keys";
const MANAGED_BY_LABEL: (&str, &str) = ("app.kubernetes.io/managed-by", "pingress-controller");

pub async fn run_host_port<F>(
    client: Client,
    shutdown_signal: F,
    namespace: String,
    node_selector: BTreeMap<String, String>,
    image_pull_secret: Option<String>,
    proxy_server_image: String,
) where
    F: Future<Output = ()> + Send + Sync + 'static,
{
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
            |i, c| async { reconcile(i, c).await },
            handle_error,
            Arc::new(Context::new(
                client,
                namespace,
                node_selector,
                image_pull_secret,
                proxy_server_image,
            )),
        )
        .log_controller_result()
        .await;
}

fn is_managed(ingress: &Ingress) -> bool {
    ingress.finalizers().iter().any(|f| f == FINALIZER)
}

/// Any one managed Ingress; reconciling any of them regenerates all managed resources
fn any_managed_ingress(store: &Store<Ingress>) -> Option<ObjectRef<Ingress>> {
    store
        .state()
        .iter()
        .find(|i| is_managed(i))
        .map(|i| ObjectRef::from_obj(i.as_ref()))
}

fn ingresses_referencing(store: &Store<Ingress>, secret: &Secret) -> Vec<ObjectRef<Ingress>> {
    let name = secret.name_any();
    let namespace = secret.namespace();
    store
        .state()
        .iter()
        .filter(|i| is_managed(i) && i.namespace() == namespace)
        .filter(|i| {
            i.spec
                .as_ref()
                .and_then(|s| s.tls.as_ref())
                .is_some_and(|tls| tls.iter().any(|t| t.secret_name.as_ref() == Some(&name)))
        })
        .map(|i| ObjectRef::from_obj(i.as_ref()))
        .collect()
}

struct Context {
    client: Client,
    namespace: String,
    node_selector: BTreeMap<String, String>,
    image_pull_secret: Option<String>,
    proxy_server_image: String,
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
    ) -> Self {
        Self {
            client,
            namespace,
            node_selector,
            image_pull_secret,
            proxy_server_image,
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

/// Base name of the TLS files of the host. Secret keys cannot contain `*`, so it is replaced by
/// `_`, which never appears in a valid Ingress host.
fn tls_file_stem(host: &str) -> String {
    host.replace('*', "_")
}

/// Key in the TLS secret (and file name in the proxy) of the certificate of the host
fn tls_cert_file(host: &str) -> String {
    format!("{}.crt", tls_file_stem(host))
}

/// Key in the TLS secret (and file name in the proxy) of the private key of the host
fn tls_key_file(host: &str) -> String {
    format!("{}.key", tls_file_stem(host))
}
