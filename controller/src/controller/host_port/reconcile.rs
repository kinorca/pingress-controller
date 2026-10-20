use crate::controller::host_port::Context;
use crate::controller::host_port::config_map::{apply_config_map, cleanup_config_map};
use crate::controller::host_port::daemonset::{apply_daemonset, cleanup_daemonset};
use crate::controller::host_port::secrets::{apply_tls_secrets, cleanup_tls_secret};
use crate::controller::ingress_class::IngressClasses;
use crate::controller::{FINALIZER, ReconcileError, is_managed, remove_finalizer};
use k8s_openapi::api::networking::v1::Ingress;
use kube::api::ListParams;
use kube::runtime::controller::Action;
use kube::runtime::finalizer;
use kube::runtime::finalizer::Event;
use kube::{Api, ResourceExt};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, instrument};

/// Managed resources are regenerated periodically to recover from missed events
const RESYNC_INTERVAL: Duration = Duration::from_secs(300);

#[instrument(skip_all, fields(namespace = ingress.namespace(), name = ingress.name_any()))]
pub(super) async fn reconcile(
    ingress: Arc<Ingress>,
    ctx: Arc<Context>,
) -> Result<Action, ReconcileError> {
    let classes = IngressClasses::load(ctx.client.clone()).await?;

    let api: Api<Ingress> = Api::namespaced(
        ctx.client.clone(),
        ingress.namespace().as_deref().unwrap_or("default"),
    );

    if !classes.is_target(&ingress) {
        // The Ingress was ours before (e.g. its class has been changed): release it and drop
        // its rules
        if is_managed(&ingress) {
            info!("Ingress is no longer a target, releasing it");
            remove_finalizer(&api, &ingress).await?;
            sync(&ctx, &classes).await?;
        }
        return Ok(Action::await_change());
    }

    if !is_managed(&ingress) && ingress.metadata.deletion_timestamp.is_none() {
        info!("Start managing Ingress");
    }

    let action = finalizer(&api, FINALIZER, ingress, |event| {
        if let Event::Cleanup(_) = event {
            info!("Ingress is being deleted, removing its rules");
        }
        async { sync(&ctx, &classes).await }
    })
    .await?;
    Ok(action)
}

/// Regenerate all managed resources from the current Ingresses
async fn sync(ctx: &Context, classes: &IngressClasses) -> Result<Action, kube::Error> {
    let result = sync_impl(ctx, classes).await;
    match result {
        Ok((ingresses, tls_hosts)) => {
            ctx.metrics.sync_succeeded(ingresses, tls_hosts);
            debug!(ingresses, tls_hosts, "Managed resources synchronized");
            Ok(Action::requeue(RESYNC_INTERVAL))
        }
        Err(e) => {
            ctx.metrics.sync_failed();
            Err(e)
        }
    }
}

/// Returns the numbers of the managed Ingresses and the hosts with TLS
async fn sync_impl(ctx: &Context, classes: &IngressClasses) -> Result<(usize, usize), kube::Error> {
    let _guard = ctx.sync_lock.lock().await;

    let api: Api<Ingress> = Api::all(ctx.client.clone());
    let ingresses = api.list(&ListParams::default()).await?;
    let ingresses: Vec<Ingress> = ingresses
        .items
        .into_iter()
        .filter(|i| i.metadata.deletion_timestamp.is_none() && classes.is_target(i))
        .collect();

    if ingresses.is_empty() {
        cleanup_daemonset(ctx).await?;
        cleanup_tls_secret(ctx).await?;
        cleanup_config_map(ctx).await?;
        return Ok((0, 0));
    }

    let tls_hosts = apply_tls_secrets(ctx, ingresses.as_slice()).await?;
    apply_config_map(ctx, ingresses.as_slice(), &tls_hosts).await?;
    apply_daemonset(ctx).await?;

    Ok((ingresses.len(), tls_hosts.len()))
}
