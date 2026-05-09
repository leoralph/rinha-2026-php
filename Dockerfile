# syntax=docker/dockerfile:1.7

### Stage 1: build da extensão Rust contra os headers do PHP 8.3
FROM --platform=linux/amd64 rust:1-slim AS extbuild

WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends \
        clang libclang-dev pkg-config ca-certificates curl gnupg \
    && rm -rf /var/lib/apt/lists/*

# php8.3-dev (Sury repo) pra ext-php-rs gerar bindings
RUN curl -sSLo /usr/share/keyrings/php.gpg https://packages.sury.org/php/apt.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/php.gpg] https://packages.sury.org/php/ bookworm main" \
        > /etc/apt/sources.list.d/php.list \
    && apt-get update && apt-get install -y --no-install-recommends \
        php8.3 php8.3-dev php8.3-cli \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTFLAGS="-C target-cpu=haswell -C target-feature=+avx2,+fma,+bmi2,+popcnt -C link-arg=-s"

COPY ext/Cargo.toml ext/Cargo.lock* ./
COPY ext/src ./src
COPY ext/resources ./resources
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release && \
    cp target/release/librinha.so /tmp/rinha.so


### Stage 2: runtime PHP 8.3 CLI + Swoole + extensão + dados baked
FROM --platform=linux/amd64 php:8.3-cli-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
        libssl-dev libcurl4-openssl-dev libc-ares-dev libbrotli-dev \
    && pecl install --configureoptions 'enable-openssl="yes" enable-swoole-curl="no" enable-cares="no" enable-brotli="no"' swoole \
    && docker-php-ext-enable swoole opcache \
    && rm -rf /var/lib/apt/lists/* /tmp/pear

COPY --from=extbuild /tmp/rinha.so /tmp/rinha.so
RUN cp /tmp/rinha.so "$(php-config --extension-dir)/rinha.so" \
    && rm /tmp/rinha.so \
    && { \
        echo 'extension=rinha.so'; \
        echo 'opcache.enable_cli=1'; \
        echo 'opcache.jit=tracing'; \
        echo 'opcache.jit_buffer_size=64M'; \
        echo 'opcache.validate_timestamps=0'; \
        echo 'opcache.max_accelerated_files=64'; \
        echo 'opcache.memory_consumption=64'; \
        echo 'memory_limit=128M'; \
    } > /usr/local/etc/php/conf.d/zz-rinha.ini

COPY src/server.php /app/server.php
COPY data/ /data/

ENV DATA_DIR=/data
WORKDIR /app

CMD ["php", "/app/server.php"]
