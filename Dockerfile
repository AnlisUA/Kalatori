FROM rust:1.82 as builder

WORKDIR /usr/src/kalatori

COPY Cargo.toml Cargo.lock ./

RUN mkdir -p src && echo "fn main() {}" > src/main.rs

RUN cargo build --release

RUN rm -rf src
COPY . .

RUN cargo build --release

FROM ubuntu:24.04

WORKDIR /app

COPY --from=builder /usr/src/kalatori/target/release/kalatori /app/kalatori

# Install CA certificates to allow HTTPS callbacks
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    update-ca-certificates && \
    rm -rf /var/lib/apt/lists/*

EXPOSE 16726

CMD ["/app/kalatori"]
