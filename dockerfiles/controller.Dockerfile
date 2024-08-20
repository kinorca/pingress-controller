FROM rust:alpine AS builder

WORKDIR /work
COPY ../controller controller
COPY ../pingress-config pingress-config

RUN --mount=type=cache,target=/work/controller/target \
    --mount=type=cache,target=/work/.cargo \
    --mount=type=cache,target=/work/pingress-config/target \
    apk add --no-cache musl-dev && \
    cd controller && \
    cargo build --release && \
    cp /work/controller/target/release/pingress-controller /pingress-controller

FROM scratch

LABEL org.opencontainers.image.source="https://github.com/kinorca/pingress-controller"
LABEL org.opencontainers.image.authors="SiLeader <sileader.dev@gmail.com>"
LABEL org.opencontainers.image.url="https://github.com/kinorca/pingress-controller"

COPY --from=builder /pingress-controller /usr/local/bin/pingress-controller

CMD ["/usr/local/bin/pingress-controller"]
