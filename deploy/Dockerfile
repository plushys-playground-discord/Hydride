# syntax=docker/dockerfile:1
# Build on the client in an amd64 OL8 container, never on the deployment server.
FROM oraclelinux:8 AS builder
ARG RUST_VERSION=1.95.0
ARG BUILD_JOBS=2
RUN dnf install -y gcc gcc-c++ make pkgconf-pkg-config fontconfig-devel freetype-devel curl ca-certificates \
    && dnf clean all
ENV CARGO_HOME=/opt/cargo RUSTUP_HOME=/opt/rustup
ENV PATH=/opt/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 --retry 3 -fsS https://sh.rustup.rs -o /tmp/rustup-init.sh \
    && sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain "$RUST_VERSION"
WORKDIR /build
COPY . .
RUN --mount=type=cache,target=/opt/cargo/registry \
    --mount=type=cache,target=/opt/cargo/git \
    --mount=type=cache,target=/build/target \
    RUSTC_WRAPPER= RUSTFLAGS='-C target-cpu=x86-64' CARGO_BUILD_JOBS="$BUILD_JOBS" \
    cargo build --locked --release --package moderation_bot --bin moderation_bot --target x86_64-unknown-linux-gnu \
    && mkdir /output \
    && cp target/x86_64-unknown-linux-gnu/release/moderation_bot /output/moderation_bot
FROM scratch AS artifact
COPY --from=builder /output/moderation_bot /moderation_bot
