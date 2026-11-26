#!/usr/bin/env bash
# Builds the static, serverless version of the browser GUI into dist/:
# the page from crates/colosseum-web/static plus the engine and the
# live-input bot as WebAssembly. Serve dist/ with any static file server
# (or Cloudflare Pages). Needs the wasm32-unknown-unknown target and a
# wasm-bindgen CLI matching the wasm-bindgen version in Cargo.lock.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
out="${1:-$root/dist}"
cd "$root"

cargo build --release --target wasm32-unknown-unknown -p colosseum-wasm
rm -rf "$out"
mkdir -p "$out/pkg"
wasm-bindgen --target web --no-typescript --out-dir "$out/pkg" \
  target/wasm32-unknown-unknown/release/colosseum_wasm.wasm
if command -v wasm-opt >/dev/null; then
  wasm-opt -Oz -o "$out/pkg/colosseum_wasm_bg.wasm" "$out/pkg/colosseum_wasm_bg.wasm"
fi

static=crates/colosseum-web/static
cp "$static/app.js" "$static/icons.js" "$static/style.css" crates/colosseum-wasm/web/local.js "$out/"
# Load the WebAssembly API before app.js boots; app.js waits on colosseumReady.
sed 's#  <script src="/icons.js"></script>#  <script>window.colosseumReady = import("./local.js").then((m) => m.start());</script>\n&#' \
  "$static/index.html" > "$out/index.html"
# Paths relative, so dist/ works from any directory.
sed -i 's#href="/style.css"#href="style.css"#; s#src="/icons.js"#src="icons.js"#; s#src="/app.js"#src="app.js"#' "$out/index.html"
echo "built $out ($(du -sh "$out" | cut -f1))"
