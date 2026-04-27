# syntax=docker/dockerfile:1.7

ARG RUST_IMAGE=rust:1-bookworm
ARG TOOLCHAIN=nightly-2023-08-25
ARG DYLINT_VERSION=2.5.0

FROM ${RUST_IMAGE} AS tooling
ARG TOOLCHAIN
ARG DYLINT_VERSION

ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_NET_GIT_FETCH_WITH_CLI=true \
    PATH=/root/.cargo/bin:${PATH}

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        clang \
        cmake \
        git \
        libdbus-1-dev \
        libssl-dev \
        libx11-dev \
        libxcb-render0-dev \
        libxcb-shape0-dev \
        libxcb-xfixes0-dev \
        libxcb1-dev \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN rustup toolchain install ${TOOLCHAIN} \
        --profile minimal \
        --component llvm-tools-preview \
        --component rust-src \
        --component rustc-dev

RUN cargo +${TOOLCHAIN} install cargo-dylint --version ${DYLINT_VERSION} --locked \
    && cargo +${TOOLCHAIN} install dylint-link --version ${DYLINT_VERSION} --locked

WORKDIR /workspace

COPY . /workspace/avail
COPY --from=sesame . /workspace/Sesame
COPY --from=scrutinizer . /workspace/scrutinizer

RUN cargo +${TOOLCHAIN} install --locked --path /workspace/scrutinizer/scrutinizer

WORKDIR /workspace/avail

RUN cargo +${TOOLCHAIN} build --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libdbus-1-3 \
        libssl3 \
        libx11-6 \
        libxcb-render0 \
        libxcb-shape0 \
        libxcb-xfixes0 \
        libxcb1 \
    && rm -rf /var/lib/apt/lists/*

ENV HOME=/data

WORKDIR /data

COPY --from=tooling /workspace/avail/target/release/avail /usr/local/bin/avail

ENTRYPOINT ["avail"]
CMD ["--help"]
