use crate::controller::load_balancer::Context;
use k8s_openapi::api::networking::v1::Ingress;
use kube::runtime::controller::Action;
use kube::runtime::finalizer;
use kube::runtime::finalizer::{Error, Event};
use kube::{Api, ResourceExt};
use std::sync::Arc;

pub(super) async fn reconcile(
    ingress: Arc<Ingress>,
    ctx: Arc<Context>,
) -> Result<Action, Error<kube::Error>> {
    if ingress
        .spec
        .as_ref()
        .and_then(|s| s.ingress_class_name.as_deref())
        .is_some_and(|s| s == "pingress")
    {
        return Ok(Action::await_change());
    }

    let api: Api<Ingress> = Api::namespaced(
        ctx.client.clone(),
        ingress.namespace().as_deref().unwrap_or("default"),
    );

    finalizer(
        &api,
        "kinorca.com/pingress-controller",
        ingress,
        |e| async { reconcile_impl(ctx, e).await },
    )
    .await
}

async fn reconcile_impl(_ctx: Arc<Context>, _event: Event<Ingress>) -> Result<Action, kube::Error> {
    todo!()
}
