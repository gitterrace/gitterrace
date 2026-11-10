# Linux image for scripts/linux-run.sh: the Linux job of lint-and-test.yml, runnable locally.
# Same base as the runner, the toolchain pinned like rust-toolchain.toml, and the tools
# `make check` needs.
# Built for the native arch (arm64 on Apple silicon), never emulated.
FROM ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852
ARG RUST_TOOLCHAIN=1.98.0
ENV DEBIAN_FRONTEND=noninteractive RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential pkg-config git curl ca-certificates \
    unzip xz-utils jq procps python3 python3-pip shellcheck shfmt make \
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
RUN pip install --break-system-packages ruff==0.17.3 \
 && cargo install cargo-machete --version 0.9.2 --locked \
 && cargo install typos-cli --version 1.51.1 --locked \
 && rm -rf /usr/local/cargo/registry /usr/local/cargo/git
RUN set -eu; \
    case "$(uname -m)" in \
      aarch64) goarch=arm64; vale=arm64; ast=aarch64; al=arm64; \
        vale_sum=3154179f4611140c9af54a8b3bc6a90bc02b73fe6c1f635369604dde0490406d; \
        gl_sum=e4a487ee7ccd7d3a7f7ec08657610aa3606637dab924210b3aee62570fb4b080; \
        al_sum=325e971b6ba9bfa504672e29be93c24981eeb1c07576d730e9f7c8805afff0c6; \
        ast_sum=73af0ebe5958acee012754a5b2826a186580ef83333092408a7a01096fc30b8b ;; \
      *) goarch=x64; vale=64-bit; ast=x86_64; al=amd64; \
        vale_sum=22aa69a556558e4a9dfc63c17030777383d8d3c48dcdfad655fa98e839d666ed; \
        gl_sum=551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb; \
        al_sum=8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8; \
        ast_sum=a7eead5552d3a8daad70f5d50d935da44bd0c138e946b5b0a96b1d38169fa639 ;; \
    esac; \
    bin=/usr/local/bin; \
    fetch() { curl -fsSL -o "/tmp/$2" "$1"; echo "$3  /tmp/$2" | sha256sum -c -; }; \
    fetch "https://github.com/vale-cli/vale/releases/download/v3.26.0/vale_3.26.0_Linux_${vale}.tar.gz" vale.tgz "$vale_sum"; \
    tar -xz -C "$bin" -f /tmp/vale.tgz vale; \
    fetch "https://github.com/gitleaks/gitleaks/releases/download/v8.30.1/gitleaks_8.30.1_linux_${goarch}.tar.gz" gitleaks.tgz "$gl_sum"; \
    tar -xz -C "$bin" -f /tmp/gitleaks.tgz gitleaks; \
    fetch "https://github.com/rhysd/actionlint/releases/download/v1.7.12/actionlint_1.7.12_linux_${al}.tar.gz" actionlint.tgz "$al_sum"; \
    tar -xz -C "$bin" -f /tmp/actionlint.tgz actionlint; \
    fetch "https://github.com/ast-grep/ast-grep/releases/download/0.51.0/app-${ast}-unknown-linux-gnu.zip" ast-grep.zip "$ast_sum"; \
    unzip -q -o /tmp/ast-grep.zip ast-grep -d "$bin"; \
    rm /tmp/vale.tgz /tmp/gitleaks.tgz /tmp/actionlint.tgz /tmp/ast-grep.zip
RUN git config --system --add safe.directory '*'
WORKDIR /work
