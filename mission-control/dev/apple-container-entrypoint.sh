#!/usr/bin/env bash

set -euo pipefail

rust_toolchain="${ALFREDO_DEV_RUST_TOOLCHAIN:-1.88.0}"
toolchain_root="/var/lib/alfredo/toolchain"
toolchain_marker="$toolchain_root/.toolchain-${rust_toolchain}"

export CARGO_HOME="$toolchain_root/cargo"
export RUSTUP_HOME="$toolchain_root/rustup"
export PATH="$CARGO_HOME/bin:$PATH"

if [[ ! -f "$toolchain_marker" ]]; then
  echo "Bootstrapping Python, Git, Bubblewrap, and Rust ${rust_toolchain}..."
  export DEBIAN_FRONTEND=noninteractive
  apt-get update
  apt-get install --yes --no-install-recommends \
    bubblewrap \
    build-essential \
    ca-certificates \
    curl \
    git \
    libssl-dev \
    pkg-config \
    python3
  rm -rf /var/lib/apt/lists/*
  mkdir -p "$CARGO_HOME" "$RUSTUP_HOME"
  curl --proto '=https' --tlsv1.2 --silent --show-error --fail \
    https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain "$rust_toolchain"
  touch "$toolchain_marker"
fi

lock_marker="node_modules/.alfredo-container-package-lock.sha256"
lock_hash="$(sha256sum package-lock.json | awk '{print $1}')"
installed_hash=""

if [[ -f "$lock_marker" ]]; then
  installed_hash="$(tr -d '\n' < "$lock_marker")"
fi

if [[ ! -x node_modules/.bin/vite || "$installed_hash" != "$lock_hash" ]]; then
  echo "Installing Linux development dependencies from package-lock.json..."
  npm ci --no-audit --no-fund
  printf '%s\n' "$lock_hash" > "$lock_marker"
fi

exec npm run dev:container
