FROM rust:1.88 as builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm src/main.rs
COPY src ./src
RUN touch src/main.rs && cargo build --release
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -u 1000 rssbot
WORKDIR /app
COPY --from=builder /app/target/release/rssbot /app/rssbot
RUN mkdir -p /app/opinionated
RUN chown -R rssbot:rssbot /app
USER rssbot
EXPOSE 8080
CMD ["./rssbot"]