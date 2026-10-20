# Pingress Controller

A Kubernetes Ingress Controller built on [Cloudflare's Pingora](https://github.com/cloudflare/pingora) high-performance reverse proxy framework.

## Overview

Pingress Controller consists of two components:

- **Controller** — watches Kubernetes Ingress resources and generates proxy configuration
- **Proxy Server** — a Pingora-based HTTP/HTTPS reverse proxy that routes traffic to backends

The controller runs as a Kubernetes Deployment and deploys the proxy server with one of two backends:

- **HostPort** (`--backend=HostPort`) — a DaemonSet of proxy server pods on every node, binding directly to host ports 80 and 443 (shown below)
- **LoadBalancer** (`--backend=LoadBalancer`) — a proxy Deployment and a Service of type `LoadBalancer` per Ingress, whose address is published to the Ingress status (see [LoadBalancer Backend](#loadbalancer-backend))

```
┌─────────────────────────────────────┐
│  Pingress Controller (Deployment)   │
│  Watches Ingress resources          │
│  Generates ConfigMap & Secret       │
│  Manages DaemonSet lifecycle        │
└──────────────┬──────────────────────┘
               │
   ┌───────────┴──────────────┐
   ▼                          ▼
ConfigMap                  Secret
(proxy.json)           (TLS certs/keys)
   │                          │
   └───────────┬──────────────┘
               ▼
┌──────────────────────────────────────┐
│  DaemonSet (on every node)           │
│  pingress-proxy-server               │
│  HTTP  → 0.0.0.0:8080 → host:80     │
│  HTTPS → 0.0.0.0:8443 → host:443    │
└──────────────────────────────────────┘
```

## Features

- **HTTP and HTTPS routing** with prefix and exact path matching
- **Wildcard domain support** (e.g., `*.example.com`)
- **SNI-based TLS** — per-host certificate selection with no restart required for cert updates
- **Dynamic configuration reload** — file system watcher detects ConfigMap/Secret changes and swaps routing rules and TLS certificates in place, without restarting the proxy
- **HTTPS redirect** — plain HTTP requests to hosts with a certificate are redirected to HTTPS (308)
- **DaemonSet deployment** — proxy runs on every node and binds to host ports for low-latency edge routing
- **LoadBalancer deployment** — a dedicated proxy and load balancer per Ingress, with `status.loadBalancer` kept up to date
- **Minimal container images** — `scratch` (controller) and distroless (proxy) based Docker images with no shell or package manager

## How It Works

### Controller (HostPort)

1. Watches all Ingress resources across the cluster that belong to an IngressClass whose `spec.controller` is `networking.kinorca.com/pingress-controller` (via `ingressClassName`, the legacy `kubernetes.io/ingress.class` annotation, or the default IngressClass)
2. Converts Ingress rules into a `PingressConfiguration` JSON structure
3. Extracts TLS certificates from referenced Kubernetes Secrets (and follows their updates, e.g. by cert-manager)
4. Creates or updates a ConfigMap and Secret in the `pingress-system` namespace
5. Creates or updates a DaemonSet to deploy proxy pods
6. Deletes the DaemonSet, ConfigMap, and Secret if no Ingresses remain

### LoadBalancer Backend

For each target Ingress, the controller creates the following resources named `pingress-<ingress name>` in the namespace of the Ingress:

| Resource | Description |
|----------|-------------|
| ConfigMap | `proxy.json` generated from the rules of the Ingress |
| Deployment | Proxy server pods (`--replicas`). The TLS secrets of the Ingress are mounted directly as a projected volume, so certificate renewal is followed without copying the secrets |
| Service | `type: LoadBalancer` exposing ports 80 and 443 (`spec.loadBalancerClass` is set by `--load-balancer-class`) |

- When the load balancer is provisioned, its IP address or hostname is copied to `status.loadBalancer.ingress` of the Ingress
- The resources are owned by the Ingress, and are deleted when the Ingress is deleted or no longer belongs to a pingress IngressClass
- Names containing `.` or longer than 63 characters are shortened, with a hash of the Ingress name appended
- Existing resources not owned by the Ingress are never deleted; applying conflicting fields fails instead of overwriting them

### Proxy Server

1. Loads routing configuration from a JSON file on startup
2. Binds HTTP (`:80`) and HTTPS (`:443`) listeners
3. Builds an in-memory TLS map keyed by hostname
4. Routes each request by matching Host header and path against the rule table
5. Watches the configuration and key directories; on change, reloads the routing table and TLS certificates without restart (the current configuration is kept if the new one cannot be loaded)

### Routing Algorithm

- Exact hostnames take precedence over wildcard hostnames, which take precedence over rules without host; the Ingress `defaultBackend` is used when nothing matches
- Wildcard hostnames (`*.example.com`) match exactly one label, as defined by the Ingress specification
- Path matching supports `Prefix` (element-wise, `/foo` matches `/foo/bar` but not `/foobar`) and `Exact` types; `ImplementationSpecific` is treated as `Prefix`. The longest path wins
- The resolved backend is `<service>.<namespace>.svc:<port>`, resolved via DNS at request time

## Deployment

### Prerequisites

- Kubernetes 1.19+ cluster
- `kubectl` configured with cluster admin access
- Container registry accessible from the cluster

### Install

```bash
# Apply RBAC and namespace (HostPort backend)
kubectl apply -f controller/manifests/hostport.yaml
# or, for the LoadBalancer backend (and change --backend in pingress-controller.yaml)
kubectl apply -f controller/manifests/load-balancer.yaml

# Create the IngressClass
kubectl apply -f controller/manifests/ingress-class.yaml

# Deploy the controller
kubectl apply -f controller/manifests/pingress-controller.yaml
```

### Controller Arguments

| Flag | Description | Default |
|------|-------------|---------|
| `--backend` | Deployment backend mode (`HostPort` or `LoadBalancer`) | required |
| `--namespace` | Namespace for proxy resources (HostPort only) | `pingress-system` |
| `--proxy-server-image` | Proxy server container image | `ghcr.io/kinorca/pingress-proxy-server:latest` |
| `--image-pull-secret` | Image pull secret name (must exist in the proxy namespace, i.e. the Ingress namespace for LoadBalancer) | — |
| `--node-selector` | Node selector labels (`key=value,...`) (HostPort only) | — |
| `--replicas` | Proxy pods per Ingress (LoadBalancer only) | `2` |
| `--load-balancer-class` | `spec.loadBalancerClass` of the proxy Services (LoadBalancer only) | — |
| `--listen-metrics` | Prometheus metrics listen address | `0.0.0.0:9090` |

### Proxy Server Arguments

| Flag | Description | Default |
|------|-------------|---------|
| `--config` | Path to proxy.json configuration file | required |
| `--watch` | Directory to watch for configuration changes (repeatable) | directory of `--config` |
| `--listen-http` | HTTP listen address | `0.0.0.0:80` |
| `--listen-https` | HTTPS listen address | `0.0.0.0:443` |
| `--listen-metrics` | Prometheus metrics listen address | `0.0.0.0:9090` |

### Logs and Metrics

Both binaries write JSON structured logs (one object per line) to stdout. The level is set with
`RUST_LOG` (default: `info`), e.g. `RUST_LOG=info,kube=warn`. The proxy writes an access log of
every request with the target `access`; disable it with `RUST_LOG=info,access=off`.

Prometheus metrics are served in the OpenMetrics text format at `/metrics` on `--listen-metrics`.

| Binary | Metric | Description |
|--------|--------|-------------|
| controller | `pingress_controller_reconciliations_total{result}` | Ingress reconciliations |
| controller | `pingress_controller_reconcile_duration_seconds` | Time taken by a reconciliation |
| controller | `pingress_controller_syncs_total{result}` | Regenerations of the managed resources (per Ingress for LoadBalancer) |
| controller | `pingress_controller_managed_ingresses` | Ingresses handled by the controller |
| controller | `pingress_controller_tls_hosts` | Hosts with an available TLS key pair |
| proxy | `pingress_proxy_http_requests_total{method,status,backend}` | HTTP requests |
| proxy | `pingress_proxy_http_request_duration_seconds{backend}` | Request duration |
| proxy | `pingress_proxy_upstream_resolve_errors_total{backend}` | Backend name resolution failures |
| proxy | `pingress_proxy_https_redirects_total` | Plain HTTP requests redirected to HTTPS |
| proxy | `pingress_proxy_tls_certificate_not_found_total` | TLS handshakes without a certificate for the SNI |
| proxy | `pingress_proxy_config_reloads_total{result}` | Configuration loads |
| proxy | `pingress_proxy_config_last_success_timestamp_seconds` | Time of the last successful configuration load |
| proxy | `pingress_proxy_routes` | Routes in the current configuration |
| proxy | `pingress_proxy_tls_certificates` | TLS certificates in the current configuration |

## Example Ingress

```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: my-ingress
  namespace: default
spec:
  ingressClassName: pingress
  tls:
    - hosts:
        - example.com
      secretName: example-tls
  rules:
    - host: example.com
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: my-service
                port:
                  number: 80
```

See `controller/manifests/sample/sample.yaml` for a complete example with a Deployment and Service.

## Proxy Configuration Format

The controller generates a JSON configuration file consumed by the proxy server:

```json
{
  "rules": [
    {
      "host": "example.com",
      "path": {
        "type": "Prefix",
        "path": "/"
      },
      "backend": {
        "type": "Service",
        "name": "my-service",
        "namespace": "default",
        "port": 80
      },
      "tls": {
        "key": "/etc/pingress/keys/example.com.key",
        "cert": "/etc/pingress/keys/example.com.crt"
      }
    }
  ],
  "default_backend": {
    "type": "Service",
    "name": "fallback",
    "namespace": "default",
    "port": 80
  }
}
```

`host` is omitted for rules without host, and `default_backend` is omitted when no Ingress has `defaultBackend`.

## Building

Building requires `protobuf` and a working Rust toolchain.

```bash
# Build all crates
cargo build --release

# Build controller image
docker build -f dockerfiles/controller.Dockerfile -t pingress-controller .

# Build proxy server image
docker build -f dockerfiles/server.Dockerfile -t pingress-proxy-server .
```

Images must be built with the repository root as the build context. The proxy server image requires `cmake` and `clang` for compiling Pingora's BoringSSL dependency, and is built on glibc because bindgen cannot load libclang from a static musl build. These are handled automatically in the multi-stage Dockerfile.

## Workspace Structure

```
pingress-controller/
├── controller/          # Kubernetes reconciliation logic
│   └── manifests/       # Kubernetes YAML manifests
├── proxy/               # Pingora-based proxy server
├── pingress-config/     # Shared configuration types
├── src/                 # Workspace root binary
└── dockerfiles/         # Dockerfiles for controller and proxy
```

## Limitations

- Backend Service endpoints are resolved via DNS at request time, not tracked dynamically
- Service ports must be specified by number; named ports are skipped
- With the HostPort backend, only one `defaultBackend` is served for the whole cluster (the first one found)
- With the LoadBalancer backend, every Ingress gets its own load balancer; Ingresses cannot share one
- The controller does not support multiple replicas with leader election

## License

Apache License 2.0. See [LICENSE](LICENSE) for details.
