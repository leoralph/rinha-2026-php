# rinha-2026-php

Solução para [Rinha de Backend 2026](https://github.com/zanfranceschi/rinha-de-backend-2026)
em **PHP (FrankenPHP, worker mode) + extensão nativa Rust**.

## Stack

- **FrankenPHP 1.10 / PHP 8.4** com OpCache + JIT tracing, em worker mode.
- **Extensão Rust** (via [`ext-php-rs`](https://github.com/davidcole1340/ext-php-rs))
  cuida de todo o caminho quente: parse de JSON, vetorização, quantização e
  busca KNN. PHP só faz o loop HTTP do worker.
- **HAProxy** com round-robin entre duas instâncias.
- **1 CPU + 350 MB** total, conforme regras (`0.45 + 0.45 + 0.10`).
- **Vetores de referência** quantizados em **int24 packed** (3 bytes/dim,
  escala 2²³−1) embarcados na imagem.

## Layout

```
src/public/index.php   loop FrankenPHP worker
ext/                   extensão Rust (mmap, VP-Tree, vectorize)
data/                  vectors.i24, labels.u8, vptree.bin (Git LFS)
Caddyfile              FrankenPHP worker config
Dockerfile             multi-stage: cargo build → runtime
docker-compose.yml     api1 + api2 + haproxy
haproxy.cfg
```

## Imagens publicadas

CI no push para `main` builda e publica em
`ghcr.io/leoralph/rinha-2026-php:latest` (e tag versionada).

## Run local

```sh
git lfs pull   # baixa data/ via LFS
docker compose up --build
curl http://localhost:9999/ready
```

## License

MIT — ver [LICENSE](./LICENSE).
