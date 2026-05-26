FROM rust:1.92.0-slim-trixie AS builder

RUN apt update && apt install -y \
    curl 7zip \
    build-essential openssl libssl-dev pkg-config libopus-dev

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
apt update && apt install -y \
    libopus-dev \
    man manpages-dev manpages-posix manpages-posix-dev
apt install --reinstall coreutils
rm -rf /var/lib/apt/lists/*
EOF

COPY --from=builder /usr/local/bin /usr/local/bin
COPY --from=builder /target/release/wobot /wobot

CMD ["/wobot"]
