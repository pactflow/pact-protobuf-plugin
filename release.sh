#!/bin/bash

if [ $# -lt 1 ]
then
    echo "Usage : $0 <Linux|Windows|macOS> <version tag>"
    exit
fi

set -e

echo Building Release for "$1" - "$2"

cargo clean
mkdir -p release_artifacts

case "$1" in
  Linux)    echo "Building for Linux"
            docker run --rm --user "$(id -u)":"$(id -g)" -v "$(pwd):/workspace" -w /workspace -t pactfoundation/rust-musl-build -c 'cargo build --release'
            gzip -c target/release/pact-protobuf-plugin > release_artifacts/pact-protobuf-plugin-linux-x86_64.gz
            openssl dgst -sha256 -r release_artifacts/pact-protobuf-plugin-linux-x86_64.gz > release_artifacts/pact-protobuf-plugin-linux-x86_64.gz.sha256
            cp pact-plugin.json release_artifacts
            NEXT=$(echo "$2" | cut -d\- -f2)
            sed -e 's/VERSION=\"0.1.5\"/VERSION=\"'${NEXT}'\"/' scripts/install-plugin.sh > release_artifacts/install-plugin.sh
            openssl dgst -sha256 -r release_artifacts/install-plugin.sh > release_artifacts/install-plugin.sh.sha256

            # Build aarch64
            cargo install cross@0.2.5
            cargo clean
            cross build --target aarch64-unknown-linux-gnu --release
            gzip -c target/aarch64-unknown-linux-gnu/release/pact-protobuf-plugin > release_artifacts/pact-protobuf-plugin-linux-aarch64.gz
            openssl dgst -sha256 -r release_artifacts/pact-protobuf-plugin-linux-aarch64.gz > release_artifacts/pact-protobuf-plugin-linux-aarch64.gz.sha256

            cargo clean
            cross build --release --target=x86_64-unknown-linux-musl
            gzip -c target/x86_64-unknown-linux-musl/release/pact-protobuf-plugin > release_artifacts/pact-protobuf-plugin-linux-musl-x86_64.gz
            openssl dgst -sha256 -r release_artifacts/pact-protobuf-plugin-linux-musl-x86_64.gz > release_artifacts/pact-protobuf-plugin-linux-musl-x86_64.gz.sha256

            echo -- Build the musl aarch64 release artifacts --
            cargo clean
            cross build --release --target=aarch64-unknown-linux-musl
            gzip -c target/aarch64-unknown-linux-musl/release/pact-protobuf-plugin > release_artifacts/pact-protobuf-plugin-linux-musl-aarch64.gz
            openssl dgst -sha256 -r release_artifacts/pact-protobuf-plugin-linux-musl-aarch64.gz > release_artifacts/pact-protobuf-plugin-linux-musl-aarch64.gz.sha256

            ;;
  Windows)  echo  "Building for Windows"
            cargo build --release
            gzip -c target/release/pact-protobuf-plugin.exe > release_artifacts/pact-protobuf-plugin-windows-x86_64.exe.gz
            openssl dgst -sha256 -r release_artifacts/pact-protobuf-plugin-windows-x86_64.exe.gz > release_artifacts/pact-protobuf-plugin-windows-x86_64.exe.gz.sha256
            ;;
  macOS)    echo  "Building for OSX"
            cargo build --release
            gzip -c target/release/pact-protobuf-plugin > release_artifacts/pact-protobuf-plugin-osx-x86_64.gz
            openssl dgst -sha256 -r release_artifacts/pact-protobuf-plugin-osx-x86_64.gz > release_artifacts/pact-protobuf-plugin-osx-x86_64.gz.sha256

            # M1
            export SDKROOT=$(xcrun -sdk macosx11.1 --show-sdk-path)
            export MACOSX_DEPLOYMENT_TARGET=$(xcrun -sdk macosx11.1 --show-sdk-platform-version)
            cargo build --target aarch64-apple-darwin --release

            gzip -c target/aarch64-apple-darwin/release/pact-protobuf-plugin > release_artifacts/pact-protobuf-plugin-osx-aarch64.gz
            openssl dgst -sha256 -r release_artifacts/pact-protobuf-plugin-osx-aarch64.gz > release_artifacts/pact-protobuf-plugin-osx-aarch64.gz.sha256
            ;;
  *)        echo "$1 is not a recognised OS"
            exit 1
            ;;
esac
