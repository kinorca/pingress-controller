# Pingress Controller

[Cloudflare の Pingora](https://github.com/cloudflare/pingora) 高性能リバースプロキシフレームワークを基盤とした Kubernetes Ingress Controller です。

## 概要

Pingress Controller は 2 つのコンポーネントで構成されています。

- **Controller** — Kubernetes の Ingress リソースを監視し、プロキシ設定を生成する
- **Proxy Server** — バックエンドへのトラフィックをルーティングする Pingora ベースの HTTP/HTTPS リバースプロキシ

Controller は Kubernetes Deployment として動作し、すべてのノードでプロキシサーバー Pod を DaemonSet として管理します。プロキシはホストポート 80 番・443 番に直接バインドします。

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
- **動的設定リロード** — ファイルシステムの変更を監視し、TLS 証明書を自動的に再読み込み
- **DaemonSet デプロイ** — 全ノードでプロキシが動作し、ホストポートへ直接バインドすることで低レイテンシを実現
- **最小限のコンテナイメージ** — OS レイヤーを持たない scratch ベースの Docker イメージ

## 動作の仕組み

### Controller

1. クラスター全体の Ingress リソースを監視し、`ingressClassName: pingress` のものを対象とする
2. Ingress ルールを内部の `PingressConfiguration` JSON 構造に変換する
3. 参照先の Kubernetes Secret から TLS 証明書を抽出する
4. `pingress-system` 名前空間の ConfigMap と Secret を作成・更新する
5. プロキシ Pod をデプロイするための DaemonSet を作成・更新する
6. Ingress が 1 つも存在しなくなった場合、DaemonSet・ConfigMap・Secret を削除する

### Proxy Server

1. 起動時に JSON ファイルからルーティング設定を読み込む
2. HTTP (`:80`) および HTTPS (`:443`) リスナーをバインドする
3. ホスト名をキーとした TLS マップをメモリ上に構築する
4. 各リクエストの Host ヘッダーとパスをルールテーブルと照合してルーティングする
5. 設定ディレクトリを監視し、変更を検知した際に TLS 証明書を再読み込みし、`SIGTERM` を送信してグレースフルリスタートを行う

### ルーティングアルゴリズム

- 完全一致ホスト名は `HashMap` で O(1) 検索
- ワイルドカードホスト名はコンパイル済みの正規表現でマッチング
- パスマッチングは `Prefix`（前方一致）と `Exact`（完全一致）に対応
- 解決されたバックエンドは `<service>.<namespace>:<port>` 形式で、リクエスト時に DNS で名前解決される

## デプロイ

### 前提条件

- Kubernetes 1.19 以上のクラスター
- クラスター管理者権限で設定された `kubectl`
- クラスターからアクセス可能なコンテナレジストリ

### インストール

```bash
# RBAC と名前空間を適用
kubectl apply -f controller/manifests/hostport.yaml

# IngressClass を作成
kubectl apply -f controller/manifests/ingress-class.yaml

# Controller をデプロイ
kubectl apply -f controller/manifests/pingress-controller.yaml
```

### Controller の引数

| フラグ | 説明 | デフォルト |
|--------|------|------------|
| `--backend` | デプロイバックエンドモード（`HostPort`） | 必須 |
| `--namespace` | プロキシリソースを配置する名前空間 | `pingress-system` |
| `--proxy-server-image` | プロキシサーバーのコンテナイメージ | 必須 |
| `--image-pull-secret` | イメージプルシークレット名 | — |
| `--node-selector` | ノードセレクターラベル（`key=value,...`） | — |

### Proxy Server の引数

| フラグ | 説明 | デフォルト |
|--------|------|------------|
| `--config` | proxy.json 設定ファイルのパス | 必須 |
| `--watch` | 設定変更を監視するディレクトリ | 必須 |
| `--listen-http` | HTTP リッスンアドレス | `0.0.0.0:80` |
| `--listen-https` | HTTPS リッスンアドレス | `0.0.0.0:443` |

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
        "cert": "/etc/pingress/keys/example.com.cert"
      }
    }
  ]
}
```

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

Proxy Server イメージは Pingora の BoringSSL 依存関係のコンパイルに `alpine-sdk`、`perl`、`cmake` を必要とします。これらはマルチステージ Dockerfile 内で自動的に処理されます。

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

- `LoadBalancer` バックエンドモードは未実装
- バックエンドの Service エンドポイントはリクエスト時に DNS で解決されるため、動的なエンドポイント追跡は行われない
- 設定のリロードはプロキシプロセスへの `SIGTERM` 送信で行われる（既存コネクションのグレースフルドレインなし）
- Controller は複数レプリカ（リーダー選出）に対応していない

## ライセンス

Apache License 2.0。詳細は [LICENSE](LICENSE) を参照してください。
