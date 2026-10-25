# SPDX-License-Identifier: GPL-2.0-or-later
# LinkScope in a container: `docker compose up -d` with compose.yaml beside this file.
#
# The web UI is prebuilt (crates/web/ui/dist) and embedded in the binary, so the build needs Rust
# only. The image holds the binary, CA certificates (for webhooks and mail over TLS) and nothing
# else, and runs as an unprivileged account.
#
# One Dockerfile for amd64 and arm64. Rust always runs on the build machine's own architecture and
# cross-compiles when the target differs; the final stage only copies files, so an image for the
# other architecture needs no emulation:
#   docker buildx build --platform linux/amd64,linux/arm64 -t ghcr.io/rinjanianalytics/linkscope:0.3.0 .
# Releases are built and pushed by .github/workflows/release.yml from a version tag.

FROM --platform=$BUILDPLATFORM rust:1.99.1-slim-bookworm AS build
ARG TARGETARCH
ARG BUILDARCH
# Jobs for cargo; lower it on a small build host (docker build --build-arg CARGO_BUILD_JOBS=2).
ARG CARGO_BUILD_JOBS=4
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS} \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
# The Rust target, and for another architecture its C compiler (ring has C) and the target.
RUN set -eu; \
    target_arch="${TARGETARCH:-$(dpkg --print-architecture)}"; build_arch="${BUILDARCH:-$(dpkg --print-architecture)}"; \
    case "$target_arch" in \
      amd64) triple=x86_64-unknown-linux-gnu; cross="gcc-x86-64-linux-gnu libc6-dev-amd64-cross" ;; \
      arm64) triple=aarch64-unknown-linux-gnu; cross="gcc-aarch64-linux-gnu libc6-dev-arm64-cross" ;; \
      *) echo "no build for $target_arch" >&2; exit 1 ;; \
    esac; \
    echo "$triple" > /target-triple; \
    if [ "$target_arch" != "$build_arch" ]; then \
      apt-get update && apt-get install -y --no-install-recommends $cross && rm -rf /var/lib/apt/lists/*; \
      rustup target add "$triple"; \
    fi
WORKDIR /src
COPY . .
# The image's files, laid out under /out: the binary (and cfgmaker by name), the CA bundle, the
# account and the directories with their owners.
RUN set -eu; triple="$(cat /target-triple)"; \
    cargo build --release --locked -p linkscope --target "$triple"; \
    install -d /out/usr/local/bin /out/etc/ssl/certs /out/etc/linkscope /out/var/lib/linkscope; \
    install -m 0755 "target/$triple/release/linkscope" /out/usr/local/bin/linkscope; \
    ln -s linkscope /out/usr/local/bin/cfgmaker; \
    cp /etc/ssl/certs/ca-certificates.crt /out/etc/ssl/certs/ca-certificates.crt; \
    cp /etc/passwd /out/etc/passwd; cp /etc/group /out/etc/group; \
    echo 'linkscope:x:10001:10001:LinkScope:/var/lib/linkscope:/usr/sbin/nologin' >> /out/etc/passwd; \
    echo 'linkscope:x:10001:' >> /out/etc/group; \
    chown root:root /out/etc/linkscope && chmod 0755 /out/etc/linkscope; \
    chown 10001:10001 /out/var/lib/linkscope && chmod 0750 /out/var/lib/linkscope

FROM debian:bookworm-slim
COPY --from=build /out/ /

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
