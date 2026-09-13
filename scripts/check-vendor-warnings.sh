#!/usr/bin/env bash
# The local UI patches are outside the workspace. Compile them as selected
# packages so their compiler warnings cannot hide among dependency warnings.
# Keep the root lockfile and patches; standalone vendor manifests are not the
# application's dependency graph. Cargo cannot override features for packages
# outside the workspace: this checks the features resolved by the application,
# not a standalone default/all-features matrix. This checks compiler warnings,
# not a new Clippy policy for the upstream code we carry.
set -euo pipefail

task_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$task_root"

for task_package in iced_core iced_wgpu; do
    printf 'Checking %s compiler warnings (resolved workspace features)\n' "$task_package"
    cargo rustc --locked -p "$task_package" --lib -- -D warnings
done
