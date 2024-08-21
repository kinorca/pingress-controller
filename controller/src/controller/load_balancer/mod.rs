mod reconcile;

use crate::controller::load_balancer::reconcile::reconcile;
use crate::controller::{handle_error, LogControllerResult};
use k8s_openapi::api::networking::v1::Ingress;
use kube::runtime::Controller;
use kube::{Api, Client};
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

pub(crate) async fn run_load_balancer<F>(
    client: Client,
    shutdown_signal: F,
    image_pull_secret: Option<String>,
    proxy_server_image: String,
) where
    F: Future<Output = ()> + Send + Sync + 'static,
{
    let ingress_api: Api<Ingress> = Api::all(client.clone());
    let ingress_wc = kube::runtime::watcher::Config::default().any_semantic();

    Controller::new(ingress_api, ingress_wc)
        .graceful_shutdown_on(shutdown_signal)
        .run(
            |i, c| async { reconcile(i, c).await },
            handle_error,
            Arc::new(Context::new(client, image_pull_secret, proxy_server_image)),
        )
        .log_controller_result()
        .await;
}

struct Context {
    client: Client,
    image_pull_secret: Option<String>,
    proxy_server_image: String,
}

impl Context {
    fn new(client: Client, image_pull_secret: Option<String>, proxy_server_image: String) -> Self {
        Self {
            client,
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
            "load-balancer".to_string(),
        ),
    ]))
}
