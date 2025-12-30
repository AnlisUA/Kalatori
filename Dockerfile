# Use Debian Bookworm for both stages to ensure glibc compatibility
FROM debian:bookworm-slim AS builder

# Install Rust and build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    clang \
    pkg-config \
    ca-certificates \
    curl \
    libssl-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.91
ENV PATH="/root/.cargo/bin:${PATH}"

# Remove old sqlite3 if present
RUN apt-get update && apt-get remove -y libsqlite3-0 libsqlite3-dev || true && rm -rf /var/lib/apt/lists/*

# Build and install SQLite 3.51.0 from source with required features for sqlx
WORKDIR /tmp
RUN curl -LO https://www.sqlite.org/2025/sqlite-autoconf-3510000.tar.gz \
    && tar xzf sqlite-autoconf-3510000.tar.gz \
    && cd sqlite-autoconf-3510000 \
    && CFLAGS="-DSQLITE_ENABLE_UNLOCK_NOTIFY=1 -DSQLITE_ENABLE_COLUMN_METADATA=1 -DSQLITE_ENABLE_DBSTAT_VTAB=1 -DSQLITE_ENABLE_FTS3=1 -DSQLITE_ENABLE_FTS3_PARENTHESIS=1 -DSQLITE_ENABLE_FTS5=1 -DSQLITE_ENABLE_JSON1=1 -DSQLITE_ENABLE_RTREE=1 -DSQLITE_ENABLE_STAT4=1" \
       ./configure --prefix=/usr/local --enable-shared --enable-static \
    && make -j$(nproc) \
    && make install \
    && ldconfig \
    && rm -rf /tmp/sqlite-autoconf*

# Create pkg-config file for sqlite3
RUN mkdir -p /usr/local/lib/pkgconfig && \
    cat > /usr/local/lib/pkgconfig/sqlite3.pc << 'EOF'
prefix=/usr/local
exec_prefix=${prefix}
libdir=${exec_prefix}/lib
includedir=${prefix}/include

Name: SQLite
Description: SQL database engine
Version: 3.51.0
Libs: -L${libdir} -lsqlite3
Libs.private: -lm -ldl -lpthread
Cflags: -I${includedir}
EOF

# Set environment for SQLite
ENV PKG_CONFIG_PATH=/usr/local/lib/pkgconfig
ENV LD_LIBRARY_PATH=/usr/local/lib
ENV SQLITE3_LIB_DIR=/usr/local/lib
ENV SQLITE3_INCLUDE_DIR=/usr/local/include

WORKDIR /usr/src/kalatori

# Copy dependency files for caching
COPY Cargo.toml Cargo.lock build.rs ./

# Create dummy source to cache dependencies
RUN mkdir -p src && echo "fn main() {}" > src/main.rs

# Build dependencies only (will fail on build.rs but that's ok for now)
RUN cargo build --release || true

# Remove dummy source
RUN rm -rf src

# Install subxt-cli
COPY Makefile ./
RUN make install-subxt-cli

# Download metadata
RUN make download-node-metadata-ci

# Copy actual source code
COPY . .

# Build the release binary
RUN CARGO_PROFILE_RELEASE_STRIP=false cargo build --release


# Runtime stage - use the same debian:bookworm-slim to ensure glibc compatibility
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy SQLite library from builder
COPY --from=builder /usr/local/lib/libsqlite3.so.0 /usr/local/lib/
RUN cd /usr/local/lib && ln -s libsqlite3.so.0 libsqlite3.so && ldconfig

# Copy the binary from builder
COPY --from=builder /usr/src/kalatori/target/release/kalatori /app/kalatori

# Expose the default port
EXPOSE 16726

CMD ["/app/kalatori"]
