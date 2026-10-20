use crate::controller::host_port::config_map::{apply_config_map, cleanup_config_map};
use crate::controller::host_port::daemonset::{apply_daemonset, cleanup_daemonset};
use crate::controller::host_port::secrets::{apply_tls_secrets, cleanup_tls_secret};
use crate::controller::host_port::{Context, is_managed};
use crate::controller::ingress_class::IngressClasses;
use crate::controller::{FINALIZER, ReconcileError};
use crate::try_with_log;
use k8s_openapi::api::networking::v1::Ingress;
use kube::api::{ListParams, Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::runtime::finalizer;
use kube::{Api, ResourceExt};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Managed resources are regenerated periodically to recover from missed events
const RESYNC_INTERVAL: Duration = Duration::from_secs(300);

pub(super) async fn reconcile(
    ingress: Arc<Ingress>,
    ctx: Arc<Context>,
) -> Result<Action, ReconcileError> {
    let classes = try_with_log!(IngressClasses::load(ctx.client.clone()).await);

    let api: Api<Ingress> = Api::namespaced(
        ctx.client.clone(),
        ingress.namespace().as_deref().unwrap_or("default"),
    );

    if !classes.is_target(&ingress) {
        // The Ingress was ours before (e.g. its class has been changed): release it and drop
        // its rules
        if is_managed(&ingress) {
            try_with_log!(remove_finalizer(&api, &ingress).await);
            sync(&ctx, &classes).await?;
        }
        return Ok(Action::await_change());
    }

    let action = finalizer(&api, FINALIZER, ingress, |_| async {
        sync(&ctx, &classes).await
    })
    .await?;
    Ok(action)
}

async fn remove_finalizer(api: &Api<Ingress>, ingress: &Ingress) -> Result<(), kube::Error> {
    let finalizers: Vec<&String> = ingress
        .finalizers()
        .iter()
        .filter(|f| *f != FINALIZER)
        .collect();
    // resourceVersion makes the patch fail if finalizers have been changed meanwhile
    let patch = json!({
        "metadata": {
            "finalizers": finalizers,
            "resourceVersion": ingress.resource_version(),
        }
    });
    api.patch(
        &ingress.name_any(),
        &PatchParams::default(),
        &Patch::Merge(patch),
    )
    .await?;
    Ok(())
}

/// Regenerate all managed resources from the current Ingresses
async fn sync(ctx: &Context, classes: &IngressClasses) -> Result<Action, kube::Error> {
    let _guard = ctx.sync_lock.lock().await;

    let api: Api<Ingress> = Api::all(ctx.client.clone());
    let ingresses = try_with_log!(api.list(&ListParams::default()).await);
    let ingresses: Vec<Ingress> = ingresses
        .items
        .into_iter()
        .filter(|i| i.metadata.deletion_timestamp.is_none() && classes.is_target(i))
        .collect();

    if ingresses.is_empty() {
        try_with_log!(cleanup_daemonset(ctx).await);
        try_with_log!(cleanup_tls_secret(ctx).await);
        try_with_log!(cleanup_config_map(ctx).await);
        return Ok(Action::requeue(RESYNC_INTERVAL));
    }

    let tls_hosts = try_with_log!(apply_tls_secrets(ctx, ingresses.as_slice()).await);
    try_with_log!(apply_config_map(ctx, ingresses.as_slice(), &tls_hosts).await);
    try_with_log!(apply_daemonset(ctx).await);

    Ok(Action::requeue(RESYNC_INTERVAL))
}
