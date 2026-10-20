use crate::controller::ingress_class::IngressClasses;
use crate::controller::load_balancer::Context;
use crate::controller::{FINALIZER, ReconcileError};
use k8s_openapi::api::networking::v1::Ingress;
use kube::runtime::controller::Action;
use kube::runtime::finalizer;
use kube::runtime::finalizer::Event;
use kube::{Api, ResourceExt};
use std::sync::Arc;

pub(super) async fn reconcile(
    ingress: Arc<Ingress>,
    ctx: Arc<Context>,
) -> Result<Action, ReconcileError> {
    let classes = IngressClasses::load(ctx.client.clone()).await?;
    if !classes.is_target(&ingress) {
        return Ok(Action::await_change());
    }

    let api: Api<Ingress> = Api::namespaced(
        ctx.client.clone(),
        ingress.namespace().as_deref().unwrap_or("default"),
    );

    let action = finalizer(&api, FINALIZER, ingress, |e| async {
        reconcile_impl(ctx, e).await
    })
    .await?;
    Ok(action)
}

async fn reconcile_impl(_ctx: Arc<Context>, _event: Event<Ingress>) -> Result<Action, kube::Error> {
    todo!()
}
