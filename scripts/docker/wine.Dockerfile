# Wine image for scripts/wine-run.sh: runs the Windows build of git-terrace and its smoke test.
# arm64 Wine runs the aarch64-pc-windows-msvc build natively on Apple silicon. (x86_64 Wine under
# Rosetta aborts in Wine's AVX state handling, so it is not used.) On an x86_64 host build this
# for linux/amd64 and run the x86_64-pc-windows-msvc exe instead.
FROM debian:trixie-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4
ENV DEBIAN_FRONTEND=noninteractive WINEDEBUG=-all WINEPREFIX=/wine XDG_RUNTIME_DIR=/tmp
RUN apt-get update \
 && apt-get install -y --no-install-recommends wine wine64 libwine git curl ca-certificates procps \
 && rm -rf /var/lib/apt/lists/*
