#!/usr/bin/env bash

set -e

source ci/rust-version.sh stable
source ci/solana-version.sh install

set -x

cargo --version
cargo install rustfilt || true
cargo install honggfuzz --version=0.5.52 --force || true
cargo install grcov --version=0.6.1 --force

cargo +"$rust_stable" build-bpf --version
