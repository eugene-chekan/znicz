#!/usr/bin/env sh
# Regenerate THIRD-PARTY-LICENSES.html (shipped in release archives) from the
# current Cargo.lock. Needs cargo-about: `cargo install --locked cargo-about`.
# CI fails when the committed file is out of date, so run this after any
# dependency change and commit the result.
set -eu
cd "$(dirname "$0")/.."
cargo fetch --locked
cargo about generate --frozen --fail about.hbs -o THIRD-PARTY-LICENSES.html
