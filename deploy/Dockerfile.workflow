# Build context is the repository root:
#   docker build -f deploy/Dockerfile.workflow .
FROM rust:1-slim AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
RUN cargo build --release -p strata-workflow

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 strata
COPY --from=builder /app/target/release/strata-workflow /usr/local/bin/strata-workflow
USER strata
EXPOSE 8081
ENTRYPOINT ["strata-workflow"]
