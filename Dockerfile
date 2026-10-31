# syntax=docker/dockerfile:1
#
# Runtime image only. CI first builds and tests, on a native Ubuntu 24.04
# runner of the same architecture:
#   target/release/kubeshelf-rs   (cargo build --locked --release)
#   web/dist/                     (npm ci --ignore-scripts && npm run build)
# The binary links only glibc/libgcc (TLS is rustls), matching this base.
#
# All runtime configuration is environment variables (KUBESHELF_*); the
# entrypoint takes no arguments. The process writes only to /work and /tmp,
# so it runs with readOnlyRootFilesystem and emptyDir volumes there.
FROM docker.io/library/ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852

# git + OpenSSH are invoked as subprocesses for the GitOps settings store.
# OpenSSH needs a passwd entry for the runtime UID.
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ca-certificates git openssh-client \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --gid 10001 kubeshelf \
 && useradd --uid 10001 --gid 10001 --home-dir /nonexistent --no-create-home --shell /usr/sbin/nologin kubeshelf \
 && install -d -o 10001 -g 10001 -m 0700 /work

COPY --chmod=0755 target/release/kubeshelf-rs /usr/local/bin/kubeshelf
COPY web/dist /usr/share/kubeshelf/web
COPY licenses /usr/share/licenses/kubeshelf

# HOME points at the writable /tmp volume; git/ssh never need a real home.
ENV KUBESHELF_STATIC_DIR=/usr/share/kubeshelf/web \
    HOME=/tmp
USER 10001:10001
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/kubeshelf"]
