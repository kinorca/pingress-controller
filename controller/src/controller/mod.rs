mod host_port;
mod ingress_class;
mod load_balancer;

use futures::{Stream, StreamExt};
pub use host_port::run_host_port;
use k8s_openapi::api::networking::v1::Ingress;
use kube::runtime::controller::Action;
use kube::runtime::reflector::ObjectRef;
pub use load_balancer::run_load_balancer;
use log::{error, info};
use std::fmt::{Display, Formatter};
use std::future::{Future, ready};
use std::sync::Arc;
use std::time::Duration;

const FINALIZER: &str = "kinorca.com/pingress-controller";

#[derive(Debug)]
enum ReconcileError {
    Kube(kube::Error),
    Finalizer(Box<kube::runtime::finalizer::Error<kube::Error>>),
}

impl Display for ReconcileError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ReconcileError::Kube(e) => write!(f, "{e}"),
            ReconcileError::Finalizer(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ReconcileError {}

impl From<kube::Error> for ReconcileError {
    fn from(value: kube::Error) -> Self {
        ReconcileError::Kube(value)
    }
}

impl From<kube::runtime::finalizer::Error<kube::Error>> for ReconcileError {
    fn from(value: kube::runtime::finalizer::Error<kube::Error>) -> Self {
        ReconcileError::Finalizer(Box::new(value))
    }
}

trait LogControllerResult {
    fn log_controller_result(self) -> impl Future<Output = ()>;
}

type KubeReconcileError =
    kube::runtime::controller::Error<ReconcileError, kube::runtime::watcher::Error>;

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

fn handle_error<C>(_ingress: Arc<Ingress>, _error: &ReconcileError, _ctx: Arc<C>) -> Action {
    Action::requeue(Duration::from_secs(5))
}
