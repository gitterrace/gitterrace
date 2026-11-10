# git terrace in a container: serve the repositories in a mounted folder, read-only.
#
#   docker build -t gitterrace .
#   docker run --rm -p 127.0.0.1:8080:8080 -v "$PWD":/repos:ro gitterrace
#
# then open http://localhost:8080. The server runs with --public: committed history only, no
# working tree, no actions, no settings writes. docs/online.md#run-with-docker has
# the details.
# Base images are pinned by digest and rustup-init by sha256, like
# deploy/test/linux.Dockerfile.

FROM debian:trixie-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4 AS build
# Keep in step with rust-toolchain.toml (scripts/docker-check.sh checks it).
ARG RUST_TOOLCHAIN=1.98.0
# Parallel compile jobs. A busy machine builds with fewer: --build-arg JOBS=1.
ARG JOBS=2
# What `git-terrace --version` prints. .git stays out of the context, so pass it:
# --build-arg GITTERRACE_VERSION="v0.1.0-$(git rev-parse --short HEAD)". Default: v0.1.0.
ARG GITTERRACE_VERSION=
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin
# The base image is pinned by digest, which fixes the package set apt resolves against.
# hadolint ignore=DL3008
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
RUN set -eu; \
    case "$(uname -m)" in \
      aarch64) t=aarch64-unknown-linux-gnu; \
        sum=e3853c5a252fca15252d07cb23a1bdd9377a8c6f3efa01531109281ae47f841c ;; \
      *) t=x86_64-unknown-linux-gnu; \
        sum=20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c ;; \
    esac; \
    curl -fsSL -o /tmp/rustup-init \
      "https://static.rust-lang.org/rustup/archive/1.28.2/${t}/rustup-init"; \
    echo "${sum}  /tmp/rustup-init" | sha256sum -c -; \
    chmod +x /tmp/rustup-init; \
    /tmp/rustup-init -y --profile minimal --default-toolchain "$RUST_TOOLCHAIN"; \
    rm /tmp/rustup-init
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p gitterrace --jobs "$JOBS" \
 && install -D target/release/git-terrace /out/git-terrace \
 && mkdir -p /out/cache /out/config /out/repos \
 && printf '[safe]\n\tdirectory = *\n' >/out/gitconfig

# The binary reads git itself (gitoxide), so the runtime needs no git: distroless with glibc.
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:08476c244888bfb425bae1ae4dd7c17f093e972aab8426883d97f3c6f280f7c1
COPY --from=build /out/git-terrace /usr/local/bin/git-terrace
# Owned by the nonroot user, so a named volume on /cache starts writable.
COPY --from=build --chown=65532:65532 /out/cache /cache
COPY --from=build --chown=65532:65532 /out/config /config
COPY --from=build /out/repos /repos
# Repositories under /repos are owned by the host user, not this one, so git's ownership check
# would refuse them. The folder is mounted read-only and served read-only, so trust it.
COPY --from=build --chmod=0644 /out/gitconfig /etc/gitconfig
# Config in /config (optional mount). The user-cache index is opt-in and only this image sets
# it. The index of a read-only repository, and the list of served repositories, go
# to /cache: mount a volume there and a restart reuses the index.
ENV XDG_CONFIG_HOME=/config XDG_CACHE_HOME=/cache XDG_STATE_HOME=/cache/state \
    GITTERRACE_INDEX_FALLBACK=/cache/index NO_COLOR=1
USER 65532:65532
EXPOSE 8080
VOLUME ["/cache"]
ENTRYPOINT ["/usr/local/bin/git-terrace", "--public", "--addr", "0.0.0.0:8080"]
CMD ["/repos"]
