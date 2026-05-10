# syntax=docker/dockerfile:1.7

FROM --platform=linux/amd64 rust:1-slim AS build
WORKDIR /src

ENV RUSTFLAGS="-C target-cpu=haswell -C target-feature=+avx2,+fma,+bmi2,+popcnt -C link-arg=-s"

COPY Cargo.toml ./
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs && \
    cargo fetch && \
    rm -rf src

COPY src ./src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release && \
    cp target/release/rinha-lb /tmp/rinha-lb && \
    strip /tmp/rinha-lb

FROM --platform=linux/amd64 debian:12-slim
COPY --from=build /tmp/rinha-lb /rinha-lb
ENTRYPOINT ["/rinha-lb"]
