### Stage 1: build da extensão Rust contra os headers do PHP do FrankenPHP.
FROM dunglas/frankenphp:1.10-php8.4-bookworm AS rustbuild

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config libclang-dev clang curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Toolchain Rust mínimo via rustup (sem mise — compatibilidade Debian)
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain stable --profile minimal

WORKDIR /build
COPY ext/Cargo.toml ext/Cargo.lock* ./
COPY ext/src ./src
COPY ext/resources ./resources
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release && \
    cp target/release/librinha.so /tmp/rinha.so


### Stage 2: runtime FrankenPHP com extensão + dados baked.
FROM dunglas/frankenphp:1.10-php8.4-bookworm

COPY --from=rustbuild /tmp/rinha.so /usr/local/lib/php/extensions/rinha.so
RUN { \
        echo 'extension=/usr/local/lib/php/extensions/rinha.so'; \
        echo 'opcache.enable=1'; \
        echo 'opcache.enable_cli=1'; \
        echo 'opcache.jit=tracing'; \
        echo 'opcache.jit_buffer_size=128M'; \
        echo 'opcache.validate_timestamps=0'; \
        echo 'opcache.max_accelerated_files=64'; \
        echo 'opcache.memory_consumption=64'; \
        echo 'opcache.preload_user=root'; \
        echo 'realpath_cache_size=4096K'; \
        echo 'realpath_cache_ttl=600'; \
        echo 'memory_limit=128M'; \
    } > /usr/local/etc/php/conf.d/rinha.ini

COPY src/public/ /app/public/
COPY Caddyfile /etc/caddy/Caddyfile
COPY data/ /data/

ENV DATA_DIR=/data \
    SERVER_NAME=:9999 \
    FRANKENPHP_CONFIG="worker /app/public/index.php"

EXPOSE 9999
