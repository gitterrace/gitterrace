# Linux image for scripts/linux-run.sh: the ubuntu job of ci.yml, runnable locally. Same base as
# the runner, the toolchain pinned like rust-toolchain.toml, and the tools `make check` needs.
# Built for the native arch (arm64 on Apple silicon), never emulated.
FROM ubuntu:24.04
ARG RUST_TOOLCHAIN=1.98.0
ENV DEBIAN_FRONTEND=noninteractive RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config git curl ca-certificates \
    unzip xz-utils jq procps python3 python3-pip shellcheck shfmt make \
 && rm -rf /var/lib/apt/lists/*
# Node 22, as on the runner: markdownlint-cli2 needs it.
RUN case "$(uname -m)" in aarch64) n=arm64 ;; *) n=x64 ;; esac \
 && curl -fsSL "https://nodejs.org/dist/v22.12.0/node-v22.12.0-linux-${n}.tar.xz" | tar -xJ -C /usr/local --strip-components=1
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$RUST_TOOLCHAIN" \
    -c clippy -c rustfmt
# Same pins as the lint tools action, with the release asset matching the image arch.
RUN pip install --break-system-packages ruff==0.17.3 \
 && cargo install cargo-machete --version 0.9.2 --locked \
 && cargo install typos-cli --version 1.51.1 --locked \
 && rm -rf /usr/local/cargo/registry /usr/local/cargo/git
RUN set -eu; \
    case "$(uname -m)" in \
      aarch64) goarch=arm64; vale=arm64; ast=aarch64; al=arm64 ;; \
      *) goarch=x64; vale=64-bit; ast=x86_64; al=amd64 ;; \
    esac; \
    bin=/usr/local/bin; \
    curl -fsSL "https://github.com/vale-cli/vale/releases/download/v3.26.0/vale_3.26.0_Linux_${vale}.tar.gz" | tar -xz -C "$bin" vale; \
    curl -fsSL "https://github.com/gitleaks/gitleaks/releases/download/v8.30.1/gitleaks_8.30.1_linux_${goarch}.tar.gz" | tar -xz -C "$bin" gitleaks; \
    curl -fsSL "https://github.com/rhysd/actionlint/releases/download/v1.7.12/actionlint_1.7.12_linux_${al}.tar.gz" | tar -xz -C "$bin" actionlint; \
    curl -fsSL -o /tmp/ast-grep.zip "https://github.com/ast-grep/ast-grep/releases/download/0.51.0/app-${ast}-unknown-linux-gnu.zip"; \
    unzip -q -o /tmp/ast-grep.zip ast-grep -d "$bin"; rm /tmp/ast-grep.zip
RUN git config --system --add safe.directory '*'
WORKDIR /work
