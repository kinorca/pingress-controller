# Build from the repository root:
#   docker build -f dockerfiles/server.Dockerfile .
#
# Built on glibc instead of musl: bindgen (used by BoringSSL) cannot load libclang from a static
# musl build script.
FROM rust:1-bookworm AS builder

RUN apt-get update && \
    apt-get install -y --no-install-recommends cmake clang libclang-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /work
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY controller controller
COPY pingress-config pingress-config
COPY proxy proxy

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/work/target \
    cargo build --release --locked --features proxy --bin proxy && \
    cp /work/target/release/proxy /pingress-proxy-server

# glibc and libstdc++ for BoringSSL
FROM gcr.io/distroless/cc-debian12

LABEL org.opencontainers.image.source="https://github.com/kinorca/pingress-controller"
LABEL org.opencontainers.image.authors="SiLeader <sileader.dev@gmail.com>"
LABEL org.opencontainers.image.url="https://github.com/kinorca/pingress-controller"

COPY --from=builder /pingress-proxy-server /usr/local/bin/pingress-proxy-server

ENTRYPOINT ["/usr/local/bin/pingress-proxy-server"]
