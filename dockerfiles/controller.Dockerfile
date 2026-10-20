# Build from the repository root:
#   docker build -f dockerfiles/controller.Dockerfile .
FROM rust:alpine AS builder

WORKDIR /work
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY controller controller
COPY pingress-config pingress-config
COPY proxy proxy

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/work/target \
    apk add --no-cache musl-dev cmake perl make g++ && \
    cargo build --release --locked --features controller --bin controller && \
    cp /work/target/release/controller /pingress-controller

FROM scratch

LABEL org.opencontainers.image.source="https://github.com/kinorca/pingress-controller"
LABEL org.opencontainers.image.authors="SiLeader <sileader.dev@gmail.com>"
LABEL org.opencontainers.image.url="https://github.com/kinorca/pingress-controller"

COPY --from=builder /pingress-controller /usr/local/bin/pingress-controller

ENTRYPOINT ["/usr/local/bin/pingress-controller"]
