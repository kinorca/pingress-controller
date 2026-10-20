# Pingress Controller

[Cloudflare の Pingora](https://github.com/cloudflare/pingora) 高性能リバースプロキシフレームワークを基盤とした Kubernetes Ingress Controller です。

## 概要

Pingress Controller は 2 つのコンポーネントで構成されています。

- **Controller** — Kubernetes の Ingress リソースを監視し、プロキシ設定を生成する
- **Proxy Server** — バックエンドへのトラフィックをルーティングする Pingora ベースの HTTP/HTTPS リバースプロキシ

Controller は Kubernetes Deployment として動作し、次の 2 つのバックエンドのいずれかでプロキシサーバーをデプロイします。

- **HostPort**（`--backend=HostPort`）— すべてのノードでプロキシサーバー Pod を DaemonSet として管理し、ホストポート 80 番・443 番に直接バインドする（下図）
- **LoadBalancer**（`--backend=LoadBalancer`）— Ingress ごとにプロキシの Deployment と `type: LoadBalancer` の Service を作成し、そのアドレスを Ingress の status に反映する（[LoadBalancer バックエンド](#loadbalancer-バックエンド) を参照）

```
┌─────────────────────────────────────┐
│  Pingress Controller (Deployment)   │
│  Ingress リソースを監視             │
│  ConfigMap と Secret を生成         │
│  DaemonSet のライフサイクルを管理   │
└──────────────┬──────────────────────┘
               │
   ┌───────────┴──────────────┐
   ▼                          ▼
ConfigMap                  Secret
(proxy.json)           (TLS 証明書/鍵)
   │                          │
   └───────────┬──────────────┘
               ▼
┌──────────────────────────────────────┐
│  DaemonSet（全ノードで動作）         │
│  pingress-proxy-server               │
│  HTTP  → 0.0.0.0:8080 → host:80     │
│  HTTPS → 0.0.0.0:8443 → host:443    │
└──────────────────────────────────────┘
```

## 機能

- **HTTP / HTTPS ルーティング** — プレフィックスおよび完全一致パスマッチング
- **ワイルドカードドメイン対応** — `*.example.com` 形式のホスト名に対応
- **SNI ベースの TLS** — ホストごとの証明書選択。証明書の更新にプロキシの再起動は不要
- **動的設定リロード** — ConfigMap / Secret の変更を監視し、プロキシを再起動せずにルーティングルールと TLS 証明書を差し替え
- **HTTPS リダイレクト** — 証明書のあるホストへの平文 HTTP リクエストを HTTPS へリダイレクト（308）
- **DaemonSet デプロイ** — 全ノードでプロキシが動作し、ホストポートへ直接バインドすることで低レイテンシを実現
- **LoadBalancer デプロイ** — Ingress ごとに専用のプロキシとロードバランサーを用意し、`status.loadBalancer` を自動で更新
- **最小限のコンテナイメージ** — シェルやパッケージマネージャーを含まない `scratch`（Controller）および distroless（Proxy）ベースの Docker イメージ

## 動作の仕組み

### Controller（HostPort）

1. クラスター全体の Ingress リソースを監視し、`spec.controller` が `networking.kinorca.com/pingress-controller` の IngressClass に属するもの（`ingressClassName`、旧来の `kubernetes.io/ingress.class` アノテーション、またはデフォルト IngressClass）を対象とする
2. Ingress ルールを内部の `PingressConfiguration` JSON 構造に変換する
3. 参照先の Kubernetes Secret から TLS 証明書を抽出する（cert-manager などによる更新にも追従する）
4. `pingress-system` 名前空間の ConfigMap と Secret を作成・更新する
5. プロキシ Pod をデプロイするための DaemonSet を作成・更新する
6. Ingress が 1 つも存在しなくなった場合、DaemonSet・ConfigMap・Secret を削除する

### LoadBalancer バックエンド

対象の Ingress ごとに、Ingress と同じ名前空間に `pingress-<Ingress 名>` という名前で次のリソースを作成します。

| リソース | 説明 |
|----------|------|
| ConfigMap | Ingress のルールから生成した `proxy.json` |
| Deployment | プロキシサーバー Pod（`--replicas`）。Ingress の TLS Secret を projected ボリュームとして直接マウントするため、Secret をコピーせずに証明書の更新へ追従する |
| Service | ポート 80・443 を公開する `type: LoadBalancer`（`--load-balancer-class` で `spec.loadBalancerClass` を指定可能） |

- ロードバランサーがプロビジョニングされると、その IP アドレスまたはホスト名を Ingress の `status.loadBalancer.ingress` に反映する
- 各リソースは Ingress を所有者とし、Ingress が削除されたとき、または pingress の IngressClass に属さなくなったときに削除される
- `.` を含む名前や 63 文字を超える名前は短縮され、Ingress 名のハッシュが付加される
- Ingress が所有していない既存のリソースは削除しない。競合するフィールドの適用は上書きせずに失敗する

### Proxy Server

1. 起動時に JSON ファイルからルーティング設定を読み込む
2. HTTP (`:80`) および HTTPS (`:443`) リスナーをバインドする
3. ホスト名をキーとした TLS マップをメモリ上に構築する
4. 各リクエストの Host ヘッダーとパスをルールテーブルと照合してルーティングする
5. 設定ディレクトリと鍵ディレクトリを監視し、変更を検知した際に再起動せずルーティングテーブルと TLS 証明書を再読み込みする（新しい設定を読み込めない場合は現在の設定を維持する）

### ルーティングアルゴリズム

- 完全一致ホスト名 → ワイルドカードホスト名 → ホスト指定なしのルールの順に優先し、どれにも一致しなければ Ingress の `defaultBackend` を使用する
- ワイルドカードホスト名（`*.example.com`）は Ingress の仕様どおり 1 ラベルのみに一致する
- パスマッチングは `Prefix`（パス要素単位。`/foo` は `/foo/bar` に一致し `/foobar` には一致しない）と `Exact`（完全一致）に対応し、`ImplementationSpecific` は `Prefix` として扱う。最も長いパスが優先される
- 解決されたバックエンドは `<service>.<namespace>.svc:<port>` 形式で、リクエスト時に DNS で名前解決される

## デプロイ

### 前提条件

- Kubernetes 1.19 以上のクラスター
- クラスター管理者権限で設定された `kubectl`
- クラスターからアクセス可能なコンテナレジストリ

### インストール

```bash
# RBAC と名前空間を適用（HostPort バックエンド）
kubectl apply -f controller/manifests/hostport.yaml
# LoadBalancer バックエンドの場合（pingress-controller.yaml の --backend も変更する）
kubectl apply -f controller/manifests/load-balancer.yaml

# IngressClass を作成
kubectl apply -f controller/manifests/ingress-class.yaml

# Controller をデプロイ
kubectl apply -f controller/manifests/pingress-controller.yaml
```

### Controller の引数

| フラグ | 説明 | デフォルト |
|--------|------|------------|
| `--backend` | デプロイバックエンドモード（`HostPort` または `LoadBalancer`） | 必須 |
| `--namespace` | プロキシリソースを配置する名前空間（HostPort のみ） | `pingress-system` |
| `--proxy-server-image` | プロキシサーバーのコンテナイメージ | `ghcr.io/kinorca/pingress-proxy-server:latest` |
| `--image-pull-secret` | イメージプルシークレット名（プロキシの名前空間、LoadBalancer では Ingress の名前空間に必要） | — |
| `--node-selector` | ノードセレクターラベル（`key=value,...`）（HostPort のみ） | — |
| `--replicas` | Ingress ごとのプロキシ Pod 数（LoadBalancer のみ） | `2` |
| `--load-balancer-class` | プロキシ Service の `spec.loadBalancerClass`（LoadBalancer のみ） | — |
| `--listen-metrics` | Prometheus メトリクスのリッスンアドレス | `0.0.0.0:9090` |

### Proxy Server の引数

| フラグ | 説明 | デフォルト |
|--------|------|------------|
| `--config` | proxy.json 設定ファイルのパス | 必須 |
| `--watch` | 設定変更を監視するディレクトリ（複数指定可） | `--config` のディレクトリ |
| `--listen-http` | HTTP リッスンアドレス | `0.0.0.0:80` |
| `--listen-https` | HTTPS リッスンアドレス | `0.0.0.0:443` |
| `--listen-metrics` | Prometheus メトリクスのリッスンアドレス | `0.0.0.0:9090` |

### ログとメトリクス

どちらのバイナリも JSON 形式の構造化ログ（1 行に 1 オブジェクト）を標準出力に出力します。ログレベルは
`RUST_LOG` で指定します（デフォルト: `info`、例: `RUST_LOG=info,kube=warn`）。プロキシは全リクエストの
アクセスログをターゲット `access` で出力します。不要な場合は `RUST_LOG=info,access=off` で無効化できます。

Prometheus メトリクスは `--listen-metrics` の `/metrics` で OpenMetrics テキスト形式で公開されます。

| バイナリ | メトリクス | 説明 |
|----------|------------|------|
| controller | `pingress_controller_reconciliations_total{result}` | Ingress の reconcile 回数 |
| controller | `pingress_controller_reconcile_duration_seconds` | reconcile の所要時間 |
| controller | `pingress_controller_syncs_total{result}` | 管理リソースの再生成回数（LoadBalancer では Ingress ごと） |
| controller | `pingress_controller_managed_ingresses` | コントローラーが扱う Ingress 数 |
| controller | `pingress_controller_tls_hosts` | TLS 鍵ペアが利用可能なホスト数 |
| proxy | `pingress_proxy_http_requests_total{method,status,backend}` | HTTP リクエスト数 |
| proxy | `pingress_proxy_http_request_duration_seconds{backend}` | リクエストの処理時間 |
| proxy | `pingress_proxy_upstream_resolve_errors_total{backend}` | バックエンドの名前解決失敗数 |
| proxy | `pingress_proxy_https_redirects_total` | HTTPS へリダイレクトした HTTP リクエスト数 |
| proxy | `pingress_proxy_tls_certificate_not_found_total` | SNI に対応する証明書がなかった TLS ハンドシェイク数 |
| proxy | `pingress_proxy_config_reloads_total{result}` | 設定の読み込み回数 |
| proxy | `pingress_proxy_config_last_success_timestamp_seconds` | 最後に設定の読み込みに成功した時刻 |
| proxy | `pingress_proxy_routes` | 現在の設定のルート数 |
| proxy | `pingress_proxy_tls_certificates` | 現在の設定の TLS 証明書数 |

## Ingress の設定例

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

Deployment と Service を含む完全な例は `controller/manifests/sample/sample.yaml` を参照してください。

## プロキシ設定ファイルの形式

Controller はプロキシサーバーが読み込む JSON 設定ファイルを生成します。

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

ホスト指定のないルールでは `host` が、`defaultBackend` を持つ Ingress がない場合は `default_backend` が省略されます。

## ビルド

ビルドには `protobuf` と Rust ツールチェーンが必要です。

```bash
# 全クレートをビルド
cargo build --release

# Controller イメージのビルド
docker build -f dockerfiles/controller.Dockerfile -t pingress-controller .

# Proxy Server イメージのビルド
docker build -f dockerfiles/server.Dockerfile -t pingress-proxy-server .
```

イメージはリポジトリのルートをビルドコンテキストとしてビルドしてください。Proxy Server イメージは Pingora の BoringSSL 依存関係のコンパイルに `cmake` と `clang` を必要とし、静的リンクの musl ビルドでは bindgen が libclang を読み込めないため glibc 上でビルドします。これらはマルチステージ Dockerfile 内で自動的に処理されます。

## ワークスペース構成

```
pingress-controller/
├── controller/          # Kubernetes 調整ロジック
│   └── manifests/       # Kubernetes YAML マニフェスト
├── proxy/               # Pingora ベースのプロキシサーバー
├── pingress-config/     # 共有設定型定義
├── src/                 # ワークスペースルートバイナリ
└── dockerfiles/         # Controller と Proxy の Dockerfile
```

## 既知の制限

- バックエンドの Service エンドポイントはリクエスト時に DNS で解決されるため、動的なエンドポイント追跡は行われない
- Service のポートは番号で指定する必要がある（名前付きポートはスキップされる）
- HostPort バックエンドでは `defaultBackend` はクラスター全体で 1 つのみ有効（最初に見つかったもの）
- LoadBalancer バックエンドでは Ingress ごとにロードバランサーが作成され、複数の Ingress で共有することはできない
- Controller は複数レプリカ（リーダー選出）に対応していない

## ライセンス

Apache License 2.0。詳細は [LICENSE](LICENSE) を参照してください。
