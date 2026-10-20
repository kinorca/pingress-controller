use k8s_openapi::api::core::v1::Service;
use k8s_openapi::api::networking::v1::{Ingress, IngressLoadBalancerIngress, IngressPortStatus};
use kube::api::{Patch, PatchParams};
use kube::{Api, ResourceExt};
use serde_json::json;
use tracing::info;

/// Load balancer addresses of the Service, in the form of the Ingress status
pub(super) fn load_balancer_ingress(service: &Service) -> Vec<IngressLoadBalancerIngress> {
    service
        .status
        .as_ref()
        .and_then(|s| s.load_balancer.as_ref())
        .and_then(|lb| lb.ingress.as_ref())
        .into_iter()
        .flatten()
        .map(|i| IngressLoadBalancerIngress {
            hostname: i.hostname.clone(),
            ip: i.ip.clone(),
            ports: i.ports.as_ref().map(|ports| {
                ports
                    .iter()
                    .map(|p| IngressPortStatus {
                        error: p.error.clone(),
                        port: p.port,
                        protocol: p.protocol.clone(),
                    })
                    .collect()
            }),
        })
        .collect()
}

/// Set `status.loadBalancer.ingress` of the Ingress if it has been changed. An empty list clears
/// it.
pub(super) async fn update_status(
    api: &Api<Ingress>,
    ingress: &Ingress,
    load_balancer: Vec<IngressLoadBalancerIngress>,
) -> Result<(), kube::Error> {
    let current = ingress
        .status
        .as_ref()
        .and_then(|s| s.load_balancer.as_ref())
        .and_then(|lb| lb.ingress.as_deref())
        .unwrap_or_default();
    if current == load_balancer.as_slice() {
        return Ok(());
    }

    info!(?load_balancer, "Updating load balancer status of Ingress");
    let ingress_status = if load_balancer.is_empty() {
        serde_json::Value::Null
    } else {
        json!(load_balancer)
    };
    let patch = json!({ "status": { "loadBalancer": { "ingress": ingress_status } } });
    api.patch_status(
        &ingress.name_any(),
        &PatchParams::default(),
        &Patch::Merge(patch),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::load_balancer_ingress;
    use k8s_openapi::api::core::v1::Service;
    use serde_json::json;

    #[test]
    fn load_balancer_ingress_from_service() {
        let service: Service = serde_json::from_value(json!({
            "status": {
                "loadBalancer": {
                    "ingress": [
                        { "ip": "192.0.2.1", "ipMode": "VIP" },
                        {
                            "hostname": "lb.example.com",
                            "ports": [{ "port": 443, "protocol": "TCP", "error": "Pending" }]
                        }
                    ]
                }
            }
        }))
        .unwrap();

        let ingress = load_balancer_ingress(&service);
        assert_eq!(ingress.len(), 2);
        assert_eq!(ingress[0].ip.as_deref(), Some("192.0.2.1"));
        assert!(ingress[0].hostname.is_none());
        assert_eq!(ingress[1].hostname.as_deref(), Some("lb.example.com"));
        let port = &ingress[1].ports.as_ref().unwrap()[0];
        assert_eq!(port.port, 443);
        assert_eq!(port.protocol, "TCP");
        assert_eq!(port.error.as_deref(), Some("Pending"));
    }

    #[test]
    fn load_balancer_ingress_pending() {
        assert!(load_balancer_ingress(&Service::default()).is_empty());
    }
}
