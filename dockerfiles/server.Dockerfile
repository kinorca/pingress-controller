FROM rust:alpine AS builder

WORKDIR /work
COPY ../proxy proxy
COPY ../pingress-config pingress-config

RUN --mount=type=cache,target=/work/proxy/target \
    --mount=type=cache,target=/work/.cargo \
    --mount=type=cache,target=/work/pingress-config/target \
    apk add --no-cache alpine-sdk perl cmake && \
    cd proxy && \
    cargo build --release && \
    cp /work/proxy/target/release/pingress-proxy-server /pingress-proxy-server

FROM scratch

LABEL authors="cerussite"

COPY --from=builder /pingress-proxy-server /usr/local/bin/pingress-proxy-server

CMD ["/usr/local/bin/pingress-proxy-server"]
