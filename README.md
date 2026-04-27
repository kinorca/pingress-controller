# Pingress Controller

A Kubernetes Ingress Controller built on [Cloudflare's Pingora](https://github.com/cloudflare/pingora) high-performance reverse proxy framework.

## Overview

Pingress Controller consists of two components:

- **Controller** — watches Kubernetes Ingress resources and generates proxy configuration
- **Proxy Server** — a Pingora-based HTTP/HTTPS reverse proxy that routes traffic to backends

The controller runs as a Kubernetes Deployment and manages a DaemonSet of proxy server pods on every node, binding directly to host ports 80 and 443.

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
- **Dynamic configuration reload** — file system watcher detects ConfigMap changes and reloads TLS certificates automatically
- **DaemonSet deployment** — proxy runs on every node and binds to host ports for low-latency edge routing
- **Minimal container images** — scratch-based Docker images with no OS layer

## How It Works

### Controller

1. Watches all Ingress resources across the cluster for `ingressClassName: pingress`
2. Converts Ingress rules into a `PingressConfiguration` JSON structure
3. Extracts TLS certificates from referenced Kubernetes Secrets
4. Creates or updates a ConfigMap and Secret in the `pingress-system` namespace
5. Creates or updates a DaemonSet to deploy proxy pods
6. Deletes the DaemonSet, ConfigMap, and Secret if no Ingresses remain

### Proxy Server

1. Loads routing configuration from a JSON file on startup
2. Binds HTTP (`:80`) and HTTPS (`:443`) listeners
3. Builds an in-memory TLS map keyed by hostname
4. Routes each request by matching Host header and path against the rule table
5. Watches the configuration directory; on change, reloads TLS certificates and sends `SIGTERM` to restart gracefully

### Routing Algorithm

- Exact hostnames are resolved via `HashMap` lookup (O(1))
- Wildcard hostnames are matched using compiled regex patterns
- Path matching supports `Prefix` and `Exact` types
- The resolved backend is `<service>.<namespace>:<port>`, resolved via DNS at request time

## Deployment

### Prerequisites

- Kubernetes 1.19+ cluster
- `kubectl` configured with cluster admin access
- Container registry accessible from the cluster

### Install

```bash
# Apply RBAC and namespace
kubectl apply -f controller/manifests/hostport.yaml

# Create the IngressClass
kubectl apply -f controller/manifests/ingress-class.yaml

# Deploy the controller
kubectl apply -f controller/manifests/pingress-controller.yaml
```

### Controller Arguments

| Flag | Description | Default |
|------|-------------|---------|
| `--backend` | Deployment backend mode (`HostPort`) | required |
| `--namespace` | Namespace for proxy resources | `pingress-system` |
| `--proxy-server-image` | Proxy server container image | required |
| `--image-pull-secret` | Image pull secret name | — |
| `--node-selector` | Node selector labels (`key=value,...`) | — |

### Proxy Server Arguments

| Flag | Description | Default |
|------|-------------|---------|
| `--config` | Path to proxy.json configuration file | required |
| `--watch` | Directory to watch for configuration changes | required |
| `--listen-http` | HTTP listen address | `0.0.0.0:80` |
| `--listen-https` | HTTPS listen address | `0.0.0.0:443` |

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
        "cert": "/etc/pingress/keys/example.com.cert"
      }
    }
  ]
}
```

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

The proxy server image requires `alpine-sdk`, `perl`, and `cmake` for compiling Pingora's BoringSSL dependency. These are handled automatically in the multi-stage Dockerfile.

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

- `LoadBalancer` backend mode is not yet implemented
- Backend Service endpoints are resolved via DNS at request time, not tracked dynamically
- Configuration reload is performed by sending `SIGTERM` to the proxy process (no graceful connection draining)
- The controller does not support multiple replicas with leader election

## License

Apache License 2.0. See [LICENSE](LICENSE) for details.
