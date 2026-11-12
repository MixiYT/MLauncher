#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
version=$(awk -F '"' '/^version = "/ { print $2; exit }' Cargo.toml)
test -n "$version"
test "$(uname -s)-$(uname -m)" = Linux-x86_64

target_dir="$PWD/target/linux-package"
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
mkdir -p dist

for edition in MLauncher MLauncher-Lite; do
    profile=release
    features=()
    readme=README.md
    if [[ "$edition" = MLauncher-Lite ]]; then
        profile=lite
        features=(--features lite)
        readme=README-Lite.md
    fi
    cargo +stable build --locked --profile "$profile" --target-dir "$target_dir" "${features[@]}"
    package="$stage/$edition"
    mkdir "$package"
    install -m 755 "$target_dir/$profile/mlauncher" "$package/$edition"
    cp "$readme" "$package/README.md"
    cp LICENSE "$package/"
    cp assets/loaders/NOTICE.txt "$package/NOTICE-Loader-Logos.txt"
    tar -czf "dist/$edition-$version-linux-x86_64.tar.gz" -C "$package" .
done
