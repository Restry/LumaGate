#!/usr/bin/env bash
# Run inside the isolated cargo-xwin Linux builder, never in the user's data directory.
set -euo pipefail
case "${1:-}" in
  x64) target=x86_64-pc-windows-msvc ;;
  arm64) target=aarch64-pc-windows-msvc ;;
  *) echo 'Usage: windows-cross-build.sh x64|arm64' >&2; exit 2 ;;
esac
export PATH="/usr/lib/llvm-19/bin:/usr/local/cargo/bin:$PATH"
if [[ "$target" == aarch64-pc-windows-msvc ]]; then
  export PATH="${PROJECT_DIR:-/work}/cross-tools:$PATH"
  # Keep the ARM64 release build within the isolated builder's memory budget.
  # Production assets, optimization, panic handling and static CRT remain enabled.
  export CARGO_PROFILE_RELEASE_LTO=false
  export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
fi
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-1.95.0}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/work/target}"
export XWIN_CACHE_DIR="${XWIN_CACHE_DIR:-/work/.xwin}"
export LIBCLANG_PATH="${LIBCLANG_PATH:-/usr/lib/llvm-19/lib}"
export RUSTFLAGS="${RUSTFLAGS:--C target-feature=+crt-static}"
cd "${PROJECT_DIR:-/work}/src-tauri"
# Match `tauri build`: embed the production frontend instead of devUrl.
cargo xwin build --release --locked --features tauri/custom-protocol --target "$target" --bin lumagate
