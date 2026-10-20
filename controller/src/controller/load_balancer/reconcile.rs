use crate::controller::ingress_class::IngressClasses;
use crate::controller::ingresses::{GetFromIngresses, TlsSecret, namespace_of};
use crate::controller::load_balancer::resources::{Resources, apply, cleanup};
use crate::controller::load_balancer::status::{load_balancer_ingress, update_status};
use crate::controller::load_balancer::{Context, resource_name};
use crate::controller::tls::ExtractTls;
use crate::controller::{FINALIZER, ReconcileError, is_managed, remove_finalizer};
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::api::networking::v1::Ingress;
use kube::runtime::controller::Action;
use kube::runtime::finalizer;
use kube::runtime::finalizer::Event;
use kube::runtime::reflector::ObjectRef;
use kube::{Api, ResourceExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, instrument, warn};

/// Managed resources are regenerated periodically to recover from missed events
const RESYNC_INTERVAL: Duration = Duration::from_secs(300);

#[instrument(skip_all, fields(namespace = ingress.namespace(), name = ingress.name_any()))]
pub(super) async fn reconcile(
    ingress: Arc<Ingress>,
    ctx: Arc<Context>,
) -> Result<Action, ReconcileError> {
    let classes = IngressClasses::load(ctx.client.clone()).await?;

    let api: Api<Ingress> = Api::namespaced(ctx.client.clone(), &namespace_of(&ingress));

    if !classes.is_target(&ingress) {
        // The Ingress was ours before (e.g. its class has been changed): release it and delete
        // its proxy
        if is_managed(&ingress) {
            info!("Ingress is no longer a target, releasing it");
            delete(&ctx, &ingress).await?;
            update_status(&api, &ingress, Vec::new()).await?;
            remove_finalizer(&api, &ingress).await?;
        }
        return Ok(Action::await_change());
    }

    if !is_managed(&ingress) && ingress.metadata.deletion_timestamp.is_none() {
        info!("Start managing Ingress");
    }

    let (ctx, api_ref) = (&ctx, &api);
    let action = finalizer(&api, FINALIZER, ingress, |event| async move {
        match event {
            Event::Apply(ingress) => sync(ctx, api_ref, &ingress).await,
            Event::Cleanup(ingress) => {
                info!("Ingress is being deleted, removing its proxy");
                delete(ctx, &ingress).await?;
                Ok(Action::await_change())
            }
        }
    })
    .await?;
    Ok(action)
}

/// Generate the proxy of the Ingress and publish its address
async fn sync(ctx: &Context, api: &Api<Ingress>, ingress: &Ingress) -> Result<Action, kube::Error> {
    match sync_impl(ctx, api, ingress).await {
        Ok(tls_hosts) => {
            ctx.sync_succeeded(ObjectRef::from_obj(ingress), Some(tls_hosts));
            debug!(tls_hosts, "Managed resources synchronized");
            Ok(Action::requeue(RESYNC_INTERVAL))
        }
        Err(e) => {
            ctx.metrics.sync_failed();
            Err(e)
        }
    }
}

/// Returns the number of the hosts with TLS
async fn sync_impl(
    ctx: &Context,
    api: &Api<Ingress>,
    ingress: &Ingress,
) -> Result<usize, kube::Error> {
    let namespace = namespace_of(ingress);
    let name = resource_name(&ingress.name_any());
    let tls = available_tls_secrets(ctx, ingress).await?;

    let resources = Resources {
        ingress,
        namespace: &namespace,
        name: &name,
        tls: &tls,
    };
    let service = apply(ctx, &resources).await?;

    let load_balancer = load_balancer_ingress(&service);
    if load_balancer.is_empty() {
        debug!(service = name, "Load balancer is not ready yet");
    }
    update_status(api, ingress, load_balancer).await?;

    Ok(tls.len())
}

async fn delete(ctx: &Context, ingress: &Ingress) -> Result<(), kube::Error> {
    let namespace = namespace_of(ingress);
    let name = resource_name(&ingress.name_any());
    cleanup(ctx, ingress, &namespace, &name).await?;
    ctx.sync_succeeded(ObjectRef::from_obj(ingress), None);
    Ok(())
}

/// TLS secrets of the Ingress that have a key pair, at most one per host (the first one wins).
/// Missing or malformed secrets are skipped so that they do not block the other hosts.
async fn available_tls_secrets(
    ctx: &Context,
    ingress: &Ingress,
) -> Result<Vec<TlsSecret>, kube::Error> {
    let api: Api<Secret> = Api::namespaced(ctx.client.clone(), &namespace_of(ingress));

    let mut available: HashMap<String, bool> = HashMap::new();
    let mut secrets: Vec<TlsSecret> = Vec::new();
    for s in std::slice::from_ref(ingress).tls_secrets() {
        if secrets.iter().any(|t| t.host == s.host) {
            warn!(
                host = s.host,
                secret_name = s.secret,
                "Host has multiple TLS secrets, ignoring this one"
            );
            continue;
        }

        let ok = match available.get(&s.secret) {
            Some(ok) => *ok,
            None => {
                let ok = match api.get_opt(&s.secret).await? {
                    Some(secret) if secret.extract_tls().is_some() => true,
                    Some(_) => {
                        warn!(
                            secret_name = s.secret,
                            "TLS secret has no tls.crt or tls.key"
                        );
                        false
                    }
                    None => {
                        warn!(secret_name = s.secret, "TLS secret is not found");
                        false
                    }
                };
                available.insert(s.secret.clone(), ok);
                ok
            }
        };
        if ok {
            secrets.push(s);
        }
    }

    Ok(secrets)
}
