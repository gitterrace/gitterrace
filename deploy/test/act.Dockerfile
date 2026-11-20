# The runner image make act-check runs ci.yml in. act's stock image runs every step as root
# with no init, and the hosted runner does neither: as root a folder never refuses a write
# and the guards read root's view, and with no init orphaned grandchildren are never reaped.
# This adds the hosted runner's user with sudo and runs as it; act-check starts it with --init.
# The user lives here, not in act's --container-options, since act passes those to a step's own
# action image too (cargo-deny), which has no such user.
FROM catthehacker/ubuntu:act-latest@sha256:a17ca1ab0676e450e5383024da90a53521edbbcf1e85b13a4d4a20bd5663eaa9
RUN useradd -m -u 1001 -G sudo runner \
    && echo 'runner ALL=(ALL) NOPASSWD:ALL' >/etc/sudoers.d/runner \
    && mkdir -p /opt/hostedtoolcache \
    && chown -R runner /opt/hostedtoolcache
USER runner
