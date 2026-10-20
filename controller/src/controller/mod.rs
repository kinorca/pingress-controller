mod host_port;
mod ingress_class;
mod load_balancer;
mod metrics;

use futures::{Stream, StreamExt};
pub use host_port::run_host_port;
use k8s_openapi::api::networking::v1::Ingress;
use kube::ResourceExt;
use kube::runtime::controller::Action;
use kube::runtime::reflector::ObjectRef;
pub use load_balancer::run_load_balancer;
pub use metrics::{Metrics, serve_metrics};
use std::fmt::{Display, Formatter};
use std::future::{Future, ready};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error};

const FINALIZER: &str = "kinorca.com/pingress-controller";

/// Delay before retrying a failed reconciliation
const RETRY_INTERVAL: Duration = Duration::from_secs(5);

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
                Ok((resource, action)) => {
                    debug!(
                        namespace = resource.namespace,
                        name = resource.name,
                        ?action,
                        "Reconciled"
                    );
                }
                // Already logged by `handle_error`
                Err(kube::runtime::controller::Error::ReconcilerFailed(..)) => {}
                Err(kube::runtime::controller::Error::ObjectNotFound(resource)) => {
                    debug!(
                        namespace = resource.namespace,
                        name = resource.name,
                        "Reconciled object no longer exists"
                    );
                }
                Err(error) => {
                    error!(error = %error, "Controller error");
                }
            }
            ready(())
        })
    }
}

fn handle_error<C>(ingress: Arc<Ingress>, error: &ReconcileError, _ctx: Arc<C>) -> Action {
    error!(
        namespace = ingress.namespace(),
        name = ingress.name_any(),
        error = %error,
        retry_after_secs = RETRY_INTERVAL.as_secs(),
        "Reconcile failed"
    );
    Action::requeue(RETRY_INTERVAL)
}
