use crate::controller::node_port::config_map::{apply_config_map, cleanup_config_map};
use crate::controller::node_port::daemonset::{apply_daemonset, cleanup_daemonset};
use crate::controller::node_port::secrets::{apply_tls_secrets, cleanup_tls_secret};
use crate::controller::node_port::{Context, INGRESS_FIELD_SELECTOR};
use k8s_openapi::api::networking::v1::Ingress;
use kube::api::ListParams;
use kube::runtime::controller::Action;
use kube::runtime::finalizer;
use kube::runtime::finalizer::{Error, Event};
use kube::Api;
use std::sync::Arc;

pub(super) async fn reconcile(
    ingress: Arc<Ingress>,
    ctx: Arc<Context>,
) -> Result<Action, Error<kube::Error>> {
    let api: Api<Ingress> = Api::all(ctx.client.clone());

    finalizer(
        &api,
        "kinorca.com/pingress-controller",
        ingress,
        |e| async { reconcile_impl(ctx, e).await },
    )
    .await
}

async fn reconcile_impl(ctx: Arc<Context>, _event: Event<Ingress>) -> Result<Action, kube::Error> {
    let api: Api<Ingress> = Api::all(ctx.client.clone());
    let ingresses = api
        .list(&ListParams::default().fields(INGRESS_FIELD_SELECTOR))
        .await?;

    if ingresses.items.is_empty() {
        cleanup_daemonset(ctx.as_ref()).await?;
        cleanup_tls_secret(ctx.as_ref()).await?;
        cleanup_config_map(ctx.as_ref()).await?;
        return Ok(Action::await_change());
    }

    apply_tls_secrets(ctx.as_ref(), ingresses.items.as_slice()).await?;
    let config_digest = apply_config_map(ctx.as_ref(), ingresses.items.as_slice()).await?;
    apply_daemonset(ctx.as_ref(), config_digest).await?;

    Ok(Action::await_change())
}
