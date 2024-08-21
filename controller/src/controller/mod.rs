mod host_port;
mod load_balancer;

use futures::{Stream, StreamExt};
pub(crate) use host_port::run_host_port;
use k8s_openapi::api::networking::v1::Ingress;
use kube::runtime::controller::Action;
use kube::runtime::reflector::ObjectRef;
pub(crate) use load_balancer::run_load_balancer;
use log::{error, info};
use std::future::{ready, Future};
use std::sync::Arc;
use std::time::Duration;

trait LogControllerResult {
    fn log_controller_result(self) -> impl Future<Output = ()>;
}

type KubeReconcileError = kube::runtime::controller::Error<
    kube::runtime::finalizer::Error<kube::Error>,
    kube::runtime::watcher::Error,
>;

impl<S> LogControllerResult for S
where
    S: Stream<Item = Result<(ObjectRef<Ingress>, Action), KubeReconcileError>>,
{
    fn log_controller_result(self) -> impl Future<Output = ()> {
        self.for_each(|res| {
            match res {
                Ok((resource, _action)) => {
                    info!(
                        "Reconcile: {} {}",
                        resource.namespace.unwrap_or("default".to_string()),
                        resource.name
                    );
                }
                Err(error) => {
                    error!("Reconcile error: {error}");
                }
            }
            ready(())
        })
    }
}

fn handle_error<C>(
    _ingress: Arc<Ingress>,
    _error: &kube::runtime::finalizer::Error<kube::Error>,
    _ctx: Arc<C>,
) -> Action {
    Action::requeue(Duration::from_secs(5))
}
