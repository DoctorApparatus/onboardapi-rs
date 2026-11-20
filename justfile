# onboardapi SDK release (crate versions may move ahead for wrapper-only fixes)
sdk_version := "9.10.0"
arch := `uname -m | sed -e 's/aarch64/armv8/' -e 's/armv7.*/armv7/'`
sdk := env_var_or_default("ONBOARDAPI_SDK_DIR", justfile_directory() / ".sdk" / sdk_version / arch + "-linux")

# Cyclone DDS profile from config/cyclonedds/: localhost | wired | wifi | unicast
dds := env_var_or_default("DDS_PROFILE", "localhost")
inspector_dir := env_var_or_default("ONBOARDAPI_INSPECTOR_DIR", parent_directory(justfile_directory()) / "Onboardapi-Inspector_External_1.3.1_onboardapi_9.9.0_ubuntu20")

export ONBOARDAPI_SDK_DIR := sdk
export CYCLONEDDS_URI := env_var_or_default("CYCLONEDDS_URI", "file://" + justfile_directory() / "config" / "cyclonedds" / dds + ".xml")
export DDKIT_CFG := env_var_or_default("DDKIT_CFG", justfile_directory() / "config" / "ddkit.json")
export LD_LIBRARY_PATH := sdk / "lib" + ":" + env_var_or_default("LD_LIBRARY_PATH", "")

# list recipes
default:
    @just --list

# download the onboardapi C++ SDK into .sdk/
fetch-sdk build="Release":
    scripts/fetch-sdk.sh {{sdk_version}} {{arch}} {{build}}

# regenerate Rust types + C++ shim from datamodel/*.rmodel
codegen:
    cargo run -q -p onboardapi-codegen -- --datamodel datamodel --workspace .
    cargo fmt --all

# parse the data model and print a summary, without writing files
check-model:
    cargo run -q -p onboardapi-codegen -- --check

build *args:
    cargo build --workspace {{args}}

# run all tests (in-process service <-> client over DDS)
test *args:
    cargo test --workspace {{args}}

lint:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings

doc:
    cargo doc -p onboardapi --no-deps --open

# simulated GNSS: `just gnss service` in one pane, `just gnss client` in another
gnss role:
    cargo run -p onboardapi --example gnss_sim -- {{role}}

# start the onboardapi-inspector with the same DDS profile (its own 9.9.0 libs)
inspector:
    cd "{{inspector_dir}}" && ./Onboardapi-Inspector.sh

# traffic for the inspector: service + client on domain 1 (inspector default)
inspector-demo domain="1":
    @case "{{domain}}" in ''|*[!0-9]*) echo "error: domain must be a number, got '{{domain}}' (usage: just inspector-demo [domain], default 1)" >&2; exit 1;; esac
    ONBOARDAPI_DOMAIN={{domain}} cargo run -p onboardapi --example inspector_demo

# print the DDS settings the recipes use
dds-info:
    @echo "CYCLONEDDS_URI=$CYCLONEDDS_URI"
    @echo "DDKIT_CFG=$DDKIT_CFG"
    @echo "inspector: {{inspector_dir}}"

# bump the crate version, commit and tag rs-v<version>; push with `git push --follow-tags`
release new_version:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ "{{new_version}}" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || { echo "error: '{{new_version}}' is not a semver version" >&2; exit 1; }
    git diff --quiet && git diff --cached --quiet || { echo "error: commit or stash your changes first" >&2; exit 1; }
    old="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
    sed -i "s/^version = \"$old\"/version = \"{{new_version}}\"/" Cargo.toml
    sed -i "s/onboardapi-sys = { path = \"crates\/onboardapi-sys\", version = \"$old\" }/onboardapi-sys = { path = \"crates\/onboardapi-sys\", version = \"{{new_version}}\" }/" Cargo.toml
    cargo update -w -q
    just lint test
    git commit -am "release onboardapi-rs {{new_version}}"
    git tag -a "rs-v{{new_version}}" -m "onboardapi-rs {{new_version}}"
    echo "tagged rs-v{{new_version}} ($old -> {{new_version}}); push with: git push --follow-tags"

# what the release workflow uploads, without uploading
publish-dry-run:
    cargo publish --workspace --exclude onboardapi-codegen --dry-run

# full pipeline: regenerate, lint, test
ci: codegen lint test
