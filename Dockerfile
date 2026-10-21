# SPDX-License-Identifier: GPL-2.0-or-later
# LinkScope in a container: `docker compose up -d` with compose.yaml beside this file.
#
# The web UI is prebuilt (crates/web/ui/dist) and embedded in the binary, so the build needs Rust
# only. The image holds the binary, CA certificates (for webhooks and mail over TLS) and nothing
# else, and runs as an unprivileged account.

FROM rust:1.92.0-slim-bookworm AS build
# Jobs for cargo; lower it on a small build host (docker build --build-arg CARGO_BUILD_JOBS=2).
ARG CARGO_BUILD_JOBS=4
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p linkscope \
    && install -m 0755 target/release/linkscope /usr/local/bin/linkscope

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 10001 linkscope \
    && useradd --system --uid 10001 --gid 10001 --home-dir /var/lib/linkscope --shell /usr/sbin/nologin linkscope \
    && install -d -m 0755 -o root -g root /etc/linkscope \
    && install -d -m 0750 -o linkscope -g linkscope /var/lib/linkscope
COPY --from=build /usr/local/bin/linkscope /usr/local/bin/linkscope
# cfgmaker by its own name too: `docker compose run --rm linkscope cfgmaker ...` works either way.
RUN ln -s /usr/local/bin/linkscope /usr/local/bin/cfgmaker

USER linkscope:linkscope
WORKDIR /var/lib/linkscope
VOLUME ["/var/lib/linkscope"]
EXPOSE 8080

# The configuration is mounted read-only at /etc/linkscope/mrtg.cfg; everything LinkScope writes
# (logs under WorkDir, the interface cache, the alert state, the lock and the PID file) goes to
# /var/lib/linkscope. The web listener answers on every container address, port 8080; compose.yaml
# publishes it on the host's loopback only (the UI has no login).
ENTRYPOINT ["linkscope"]
CMD ["--daemon", "--foreground", \
     "--listen", "0.0.0.0:8080", \
     "--confcache-file", "/var/lib/linkscope/linkscope.ok", \
     "--lock-file", "/var/lib/linkscope/linkscope.lock", \
     "--pid-file", "/var/lib/linkscope/linkscope.pid", \
     "/etc/linkscope/mrtg.cfg"]

# /healthz over the container's loopback, with bash's /dev/tcp (no curl in the image).
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD ["bash", "-c", "exec 3<>/dev/tcp/127.0.0.1/8080 && printf 'GET /healthz HTTP/1.0\\r\\nHost: localhost\\r\\n\\r\\n' >&3 && grep -q '^ok' <&3"]

LABEL org.opencontainers.image.title="LinkScope" \
      org.opencontainers.image.description="Link utilization monitoring: a memory-safe drop-in reimplementation of MRTG with a built-in web UI" \
      org.opencontainers.image.vendor="Rinjani Analytics" \
      org.opencontainers.image.licenses="GPL-2.0-or-later" \
      org.opencontainers.image.url="https://rinjanianalytics.com" \
      org.opencontainers.image.source="https://github.com/rinjanianalytics/linkscope"
