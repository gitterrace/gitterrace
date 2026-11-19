# Linux image for scripts/dev/linux-run.sh: the Linux job of lint-and-test.yml, runnable locally.
# Same base as the runner, the toolchain pinned like rust-toolchain.toml, and the tools
# `make check` needs.
# Built for the native arch (arm64 on Apple silicon), never emulated.
FROM ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852
ARG RUST_TOOLCHAIN=1.98.0
ENV DEBIAN_FRONTEND=noninteractive RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config git curl ca-certificates \
    unzip xz-utils jq procps python3 python3-pip make \
 && rm -rf /var/lib/apt/lists/*
# Node 22, as on the runner: markdownlint-cli2 needs it.
RUN set -eu; \
    case "$(uname -m)" in \
      aarch64) n=arm64; sum=8cfd5a8b9afae5a2e0bd86b0148ca31d2589c0ea669c2d0b11c132e35d90ed68 ;; \
      *) n=x64; sum=22982235e1b71fa8850f82edd09cdae7e3f32df1764a9ec298c72d25ef2c164f ;; \
    esac; \
    curl -fsSL -o /tmp/node.tar.xz "https://nodejs.org/dist/v22.12.0/node-v22.12.0-linux-${n}.tar.xz"; \
    echo "${sum}  /tmp/node.tar.xz" | sha256sum -c -; \
    tar -xJ -C /usr/local --strip-components=1 -f /tmp/node.tar.xz; rm /tmp/node.tar.xz
# rustup-init from the pinned rustup release, checked against its published sha256.
RUN set -eu; \
    case "$(uname -m)" in \
      aarch64) t=aarch64-unknown-linux-gnu; sum=e3853c5a252fca15252d07cb23a1bdd9377a8c6f3efa01531109281ae47f841c ;; \
      *) t=x86_64-unknown-linux-gnu; sum=20a06e644b0d9bd2fbdbfd52d42540bdde820ea7df86e92e533c073da0cdd43c ;; \
    esac; \
    curl -fsSL -o /tmp/rustup-init "https://static.rust-lang.org/rustup/archive/1.28.2/${t}/rustup-init"; \
    echo "${sum}  /tmp/rustup-init" | sha256sum -c -; \
    chmod +x /tmp/rustup-init; \
    /tmp/rustup-init -y --profile minimal --default-toolchain "$RUST_TOOLCHAIN" -c clippy -c rustfmt; \
    rm /tmp/rustup-init
# Same pins as the lint tools action, with the release asset matching the image arch.
RUN pip install --break-system-packages ruff==0.17.5 \
 && cargo install cargo-machete --version 0.9.2 --locked \
 && cargo install typos-cli --version 1.52.2 --locked \
 && rm -rf /usr/local/cargo/registry /usr/local/cargo/git
# The lint tools CI's lint-tools action installs, by its own script and pins file (passed as
# the build context "lint-tools" by scripts/dev/linux-run.sh), for the image arch.
COPY --from=lint-tools linux-install.sh linux-pins.txt /tmp/lint-tools/
RUN bash /tmp/lint-tools/linux-install.sh /usr/local/bin && rm -rf /tmp/lint-tools
RUN git config --system --add safe.directory '*'
WORKDIR /work
