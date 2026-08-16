FROM rust:1.92.0-slim-trixie AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    curl 7zip \
    build-essential openssl libssl-dev pkg-config libopus-dev && \
    rm -rf /var/lib/apt/lists/*

# build dependencies first
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && \
    echo "// dummy file" > src/lib.rs && \
    cargo build --release --locked && \
    rm -rf src

# rebuild with actual source
COPY src ./src
COPY migrations ./migrations
COPY .sqlx ./.sqlx

RUN cargo build --release --locked

RUN curl -fsSL https://deno.land/install.sh | DENO_INSTALL=/usr/local sh
RUN curl -L https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -o /usr/local/bin/yt-dlp && \
    chmod a+rx /usr/local/bin/yt-dlp

FROM debian:trixie-slim

RUN <<EOF
set -e
# allow manpage installation
sed -i '/path-exclude \/usr\/share\/man/d' /etc/dpkg/dpkg.cfg.d/docker
sed -i '/path-exclude \/usr\/share\/groff/d' /etc/dpkg/dpkg.cfg.d/docker

# add non-free
sed -i 's/Components: main/Components: main non-free/' /etc/apt/sources.list.d/debian.sources

# dependencies + manpages
apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libopus0 \
    man manpages-dev manpages-posix manpages-posix-dev
apt-get install -y --reinstall --no-install-recommends coreutils
rm -rf /var/lib/apt/lists/* /var/cache/apt/archives/*

# create non-root user and group
groupadd --system --gid 10001 wobot
useradd --system --uid 10001 --gid wobot --no-create-home --shell /usr/sbin/nologin wobot
EOF

WORKDIR /app

COPY --from=builder /usr/local/bin /usr/local/bin
COPY --from=builder /target/release/wobot /app/wobot

USER wobot:wobot

CMD ["/app/wobot"]
