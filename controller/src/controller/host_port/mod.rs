mod config_map;
mod daemonset;
mod ingresses;
mod reconcile;
mod secrets;

use crate::controller::host_port::reconcile::reconcile;
use crate::controller::{handle_error, LogControllerResult};
use k8s_openapi::api::apps::v1::DaemonSet;
use k8s_openapi::api::core::v1::Service;
use k8s_openapi::api::networking::v1::Ingress;
use kube::runtime::Controller;
use kube::{Api, Client};
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

const FIELD_MANAGER: &str = "kinorca.com/pingress-controller";
const TLS_SECRET_NAME: &str = "pingress-tls-secret";
const CONFIG_MAP_NAME: &str = "pingress-config";
const CONFIG_KEY: &str = "proxy.json";
const SECRET_BASE_PATH: &str = "/etc/pingress/keys";

pub(crate) async fn run_host_port<F>(
    client: Client,
    shutdown_signal: F,
    namespace: String,
    node_selector: Vec<String>,
    image_pull_secret: Option<String>,
    proxy_server_image: String,
) where
    F: Future<Output = ()> + Send + Sync + 'static,
{
    let node_selector = node_selector
        .into_iter()
        .map(|n| {
            let (k, v) = n.split_once("=").unwrap();
            (k.to_string(), v.to_string())
        })
        .collect();

    let ingress_api: Api<Ingress> = Api::all(client.clone());
    let ingress_wc = kube::runtime::watcher::Config::default().any_semantic();

    let daemonset_api: Api<DaemonSet> = Api::namespaced(client.clone(), namespace.as_str());
    let daemonset_wc = kube::runtime::watcher::Config::default()
        .labels("kinorca.com/managed-by=pingress-controller");

    let service_api: Api<Service> = Api::namespaced(client.clone(), namespace.as_str());
    let service_wc = kube::runtime::watcher::Config::default()
        .labels("kinorca.com/managed-by=pingress-controller");

    Controller::new(ingress_api, ingress_wc)
        .graceful_shutdown_on(shutdown_signal)
        .owns(daemonset_api, daemonset_wc)
        .owns(service_api, service_wc)
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

struct Context {
    client: Client,
    namespace: String,
    node_selector: BTreeMap<String, String>,
    image_pull_secret: Option<String>,
    proxy_server_image: String,
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
        }
    }
}

fn manifest_labels() -> Option<BTreeMap<String, String>> {
    Some(BTreeMap::from([
        (
            "app.kubernetes.io/managed-by".to_string(),
            "pingress-controller".to_string(),
        ),
        (
            "kinorca.com/pingress-controller-type".to_string(),
            "host-port".to_string(),
        ),
    ]))
}
