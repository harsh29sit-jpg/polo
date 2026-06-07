FROM rust:1.79-slim-bookworm AS builder

WORKDIR /build

# Cache dependency compilation by copying manifests first
COPY Cargo.toml Cargo.lock ./
COPY pkg/polo-core/Cargo.toml   pkg/polo-core/
COPY pkg/polo-client/Cargo.toml pkg/polo-client/
COPY internal/polo-store/Cargo.toml  internal/polo-store/
COPY internal/polo-server/Cargo.toml internal/polo-server/
COPY cmd/polo/Cargo.toml   cmd/polo/
COPY cmd/polod/Cargo.toml  cmd/polod/

# Stub out lib/main so cargo can fetch and compile deps without full source
RUN mkdir -p pkg/polo-core/src pkg/polo-client/src \
             internal/polo-store/src internal/polo-server/src \
             cmd/polo/src cmd/polod/src && \
    echo "pub fn stub(){}" > pkg/polo-core/src/lib.rs && \
    echo "pub fn stub(){}" > pkg/polo-client/src/lib.rs && \
    echo "pub fn stub(){}" > internal/polo-store/src/lib.rs && \
    echo "pub fn stub(){}" > internal/polo-server/src/lib.rs && \
    echo "fn main(){}"     > cmd/polo/src/main.rs && \
    echo "fn main(){}"     > cmd/polod/src/main.rs

RUN cargo build --release 2>&1 | tail -5 || true

# Build for real
COPY . .
RUN touch pkg/polo-core/src/lib.rs pkg/polo-client/src/lib.rs \
          internal/polo-store/src/lib.rs internal/polo-server/src/lib.rs \
          cmd/polo/src/main.rs cmd/polod/src/main.rs && \
    cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -u 1000 polo
USER polo
WORKDIR /home/polo

COPY --from=builder /build/target/release/polod /usr/local/bin/polod
COPY --from=builder /build/target/release/polo  /usr/local/bin/polo

EXPOSE 5432

VOLUME ["/data"]

ENTRYPOINT ["polod"]
CMD ["--db", "/data/polo.db", "--addr", "0.0.0.0:5432"]
