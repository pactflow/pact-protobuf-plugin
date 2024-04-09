#!/usr/bin/env sh

set -e

export LIBZ_SYS_STATIC=1
cargo build
./target/debug/pact-protobuf-plugin -v
