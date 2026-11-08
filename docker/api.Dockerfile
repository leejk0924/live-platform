# syntax=docker/dockerfile:1

FROM rust:1.99.1-bookworm@sha256:7085845219c18e7c5b1f34ed9ce89b84f822f2e3d54a02774cedc996ac0f304f AS builder
WORKDIR /src
# Mount points of the recording spool and store. Docker fills a new named volume from the
# image's directory, owner included, so the non-root user can write to it.
RUN mkdir /recordings /recording-store
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bin api \
    && cp target/release/api /api

FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f
COPY --from=builder /api /app/api
COPY --from=builder --chown=65532:65532 /recordings /recordings
COPY --from=builder --chown=65532:65532 /recording-store /recording-store
USER nonroot
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 CMD ["/app/api", "healthcheck"]
ENTRYPOINT ["/app/api"]
