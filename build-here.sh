#!/bin/sh
# Rebuilds Account Switch into a real horizon-client binary on this host.
#
# The launcher does this itself on the player's machine (`horizon-launcher
# native-build <bundle> --trust-native-code`); this script is the same steps by
# hand, for checking the bundle without a launcher install.
#
# Needs: Rust (rustup), build-essential, pkg-config, libasound2-dev, libudev-dev,
# the Horizon source at the commit the bundle pins, and horizon-mixins.
#
#   ./build-here.sh /tmp/horizon /tmp/horizon-mixins /tmp/out
set -eu

HORIZON=${1:?the Horizon checkout}
MIXINS=${2:?the horizon-mixins checkout}
OUT=${3:?an empty output folder}
BUNDLE=$(cd "$(dirname "$0")" && pwd)
STAGE="$OUT/game"
WORK="$OUT/mods"

# The bundle, as the tool wants it: a folder of bundles, without a Cargo.lock or
# a target/ from a local `cargo test` (a bundle crate builds against Horizon's).
rm -rf "$WORK"
mkdir -p "$WORK/account-switch"
cp -r "$BUNDLE/." "$WORK/account-switch/"
rm -rf "$WORK/account-switch/core/Cargo.lock" "$WORK/account-switch/core/target"

# horizon-mixins applies the bundle to a copy of the source.
if [ ! -x "$MIXINS/target/debug/horizon-mixins" ]; then
    cargo build --locked --manifest-path "$MIXINS/Cargo.toml" -p horizon-mixins-cli
fi
mkdir -p "$OUT"
# The workspace must be symlink-free (the launcher builds from GitHub's archive,
# which has no links): copy without them.
rm -rf "$OUT/source"
cp -a "$HORIZON" "$OUT/source"
find "$OUT/source" -type l -delete
"$MIXINS/target/debug/horizon-mixins" apply \
    --workspace "$OUT/source" --mods "$WORK" --out "$STAGE"

# Compile: the launcher's profile and package.
cd "$STAGE"
CARGO_TARGET_DIR="$OUT/target" cargo build --locked --profile fast -p horizon-client
echo "built: $OUT/target/fast/horizon-client"
