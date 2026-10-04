#!/usr/bin/env bash
# Builds llama.cpp's server as one self-contained binary (static, Metal shaders embedded) and
# puts it where Tauri bundles sidecars: src-tauri/binaries/llama-server-<target-triple>.
# Murmur's assistant runs it in the background as its built-in AI; nothing has to be installed separately.
set -euo pipefail

VERSION="v0.5.0"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
OUT="$ROOT/src-tauri/binaries/llama-server-$TRIPLE"
CACHE="$ROOT/src-tauri/target/llama.cpp-$VERSION"

if [[ -x "$OUT" && "$(cat "$OUT.version" 2>/dev/null)" == "$VERSION" ]]; then
  echo "llama-server $VERSION already built"
  exit 0
fi

# Check for the sources themselves, not just the folder: CI's Rust cache restores target/ with
# files it doesn't know about pruned, leaving an empty llama.cpp folder behind.
if [[ ! -f "$CACHE/CMakeLists.txt" ]]; then
  rm -rf "$CACHE"
  mkdir -p "$CACHE"
  curl -fsSL "https://github.com/ggml-org/llama.cpp/archive/refs/tags/$VERSION.tar.gz" | tar xz -C "$CACHE" --strip-components 1
fi

cmake -S "$CACHE" -B "$CACHE/build" \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_OSX_DEPLOYMENT_TARGET=11.0 \
  -DBUILD_SHARED_LIBS=OFF \
  -DGGML_METAL=ON \
  -DGGML_METAL_EMBED_LIBRARY=ON \
  -DGGML_NATIVE=OFF \
  -DLLAMA_CURL=OFF \
  -DLLAMA_OPENSSL=OFF \
  -DGGML_BLAS=OFF \
  -DLLAMA_BUILD_TESTS=OFF \
  -DLLAMA_BUILD_EXAMPLES=OFF \
  -DLLAMA_BUILD_SERVER=ON > /dev/null
cmake --build "$CACHE/build" --config Release --target llama-server -j "$(sysctl -n hw.ncpu)" > /dev/null

mkdir -p "$(dirname "$OUT")"
cp "$CACHE/build/bin/llama-server" "$OUT"
echo "$VERSION" > "$OUT.version"
echo "built $OUT"
