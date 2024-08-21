use crate::controller::host_port::config_map::{apply_config_map, cleanup_config_map};
use crate::controller::host_port::daemonset::{apply_daemonset, cleanup_daemonset};
use crate::controller::host_port::secrets::{apply_tls_secrets, cleanup_tls_secret};
use crate::controller::host_port::Context;
use crate::try_with_log;
use k8s_openapi::api::networking::v1::Ingress;
use kube::api::ListParams;
use kube::runtime::controller::Action;
use kube::runtime::finalizer;
use kube::runtime::finalizer::{Error, Event};
use kube::{Api, ResourceExt};
use std::sync::Arc;

pub(super) async fn reconcile(
    ingress: Arc<Ingress>,
    ctx: Arc<Context>,
) -> Result<Action, Error<kube::Error>> {
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

async fn reconcile_impl(ctx: Arc<Context>, _event: Event<Ingress>) -> Result<Action, kube::Error> {
    let api: Api<Ingress> = Api::all(ctx.client.clone());
    let ingresses = try_with_log!(api.list(&ListParams::default()).await);
    let ingresses: Vec<Ingress> = ingresses
        .items
        .into_iter()
        .filter(|i| {
            i.spec.as_ref().is_some_and(|s| {
                s.ingress_class_name
                    .as_ref()
                    .is_some_and(|c| c == "pingress")
            })
        })
        .collect();

    if ingresses.is_empty() {
        try_with_log!(cleanup_daemonset(ctx.as_ref()).await);
        try_with_log!(cleanup_tls_secret(ctx.as_ref()).await);
        try_with_log!(cleanup_config_map(ctx.as_ref()).await);
        return Ok(Action::await_change());
    }

    try_with_log!(apply_tls_secrets(ctx.as_ref(), ingresses.as_slice()).await);
    try_with_log!(apply_config_map(ctx.as_ref(), ingresses.as_slice()).await);
    try_with_log!(apply_daemonset(ctx.as_ref()).await);

    Ok(Action::await_change())
}
