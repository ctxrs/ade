#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo_root"

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/buildbuddy-bootstrap-contract.XXXXXX")"
trap 'rm -rf "$tmpdir"' EXIT

render_linux() {
  env \
    BUILDBUDDY_APP_TARGET="grpcs://remote.buildbuddy.io" \
    BUILDBUDDY_EXECUTOR_API_KEY="linux-key" \
    BUILDBUDDY_EXECUTOR_CONFIG_ROOT="/etc/buildbuddy-release" \
    BUILDBUDDY_EXECUTOR_STATE_ROOT="/var/lib/buildbuddy-release" \
    BUILDBUDDY_EXECUTOR_MAX_RUNNER_MEMORY_USAGE_BYTES="4000000000" \
    BUILDBUDDY_EXECUTOR_MAX_TOTAL_MEMORY_USAGE_BYTES="12000000000" \
    BUILDBUDDY_EXECUTOR_ROOT_DIRECTORY="/var/lib/buildbuddy/remotebuilds" \
    BUILDBUDDY_EXECUTOR_LOCAL_CACHE_DIRECTORY="/var/lib/buildbuddy/filecache" \
    BUILDBUDDY_EXECUTOR_LOCAL_CACHE_SIZE_BYTES="200000000000" \
    MY_POOL="linux-amd64-default" \
    envsubst <"scripts/buildbuddy/templates/linux-executor.config.yaml.tmpl" >"$tmpdir/linux-config.yaml"

  env \
    BUILDBUDDY_EXECUTOR_CONFIG_ROOT="/etc/buildbuddy-release" \
    BUILDBUDDY_EXECUTOR_STATE_ROOT="/var/lib/buildbuddy-release" \
    envsubst <"scripts/buildbuddy/systemd/buildbuddy-executor.service.tmpl" >"$tmpdir/linux.service"
}

render_mac() {
  env \
    BUILDBUDDY_APP_TARGET="grpcs://remote.buildbuddy.io" \
    BUILDBUDDY_EXECUTOR_API_KEY="mac-key" \
    BUILDBUDDY_EXECUTOR_STATE_ROOT="/Users/example-user/buildbuddy" \
    BUILDBUDDY_EXECUTOR_ROOT_DIRECTORY="/Users/example-user/buildbuddy/remotebuilds" \
    BUILDBUDDY_EXECUTOR_LOCAL_CACHE_DIRECTORY="/Users/example-user/buildbuddy/filecache" \
    BUILDBUDDY_EXECUTOR_LOCAL_CACHE_SIZE_BYTES="100000000000" \
    MY_POOL="darwin-arm64-default" \
    envsubst <"scripts/buildbuddy/templates/mac-executor.config.yaml.tmpl" >"$tmpdir/mac-config.yaml"

  env \
    BUILDBUDDY_APP_TARGET="grpcs://remote.buildbuddy.io" \
    BUILDBUDDY_EXECUTOR_API_KEY="mac-key" \
    BUILDBUDDY_EXECUTOR_STATE_ROOT="/Users/example-user/buildbuddy" \
    BUILDBUDDY_EXECUTOR_ROOT_DIRECTORY="/Users/example-user/buildbuddy/remotebuilds" \
    BUILDBUDDY_EXECUTOR_LOCAL_CACHE_DIRECTORY="/Users/example-user/buildbuddy/filecache" \
    BUILDBUDDY_EXECUTOR_LOCAL_CACHE_SIZE_BYTES="100000000000" \
    MY_POOL="darwin-arm64-default" \
    envsubst <"scripts/buildbuddy/templates/buildbuddy-executor.plist.tmpl" >"$tmpdir/mac.plist"
}

render_linux
render_mac

rg -F 'executor_tmp="$(mktemp /tmp/buildbuddy-executor.' "scripts/buildbuddy/bootstrap_linux_executor.sh" >/dev/null
rg -F 'mv "${executor_tmp}" /usr/local/bin/buildbuddy-executor' "scripts/buildbuddy/bootstrap_linux_executor.sh" >/dev/null
rg -F 'executor_tmp="$(mktemp "${TMPDIR:-/tmp}/buildbuddy-executor.' "scripts/buildbuddy/bootstrap_darwin_executor.sh" >/dev/null
rg -F 'mv "${executor_tmp}" "${BUILDBUDDY_EXECUTOR_STATE_ROOT}/bin/buildbuddy-executor"' "scripts/buildbuddy/bootstrap_darwin_executor.sh" >/dev/null

rg -F 'app_target: "grpcs://remote.buildbuddy.io"' "$tmpdir/linux-config.yaml" >/dev/null
rg -F 'root_directory: "/var/lib/buildbuddy/remotebuilds"' "$tmpdir/linux-config.yaml" >/dev/null
rg -F 'local_cache_directory: "/var/lib/buildbuddy/filecache"' "$tmpdir/linux-config.yaml" >/dev/null
rg -F 'docker_socket: /var/run/docker.sock' "$tmpdir/linux-config.yaml" >/dev/null
rg -F 'docker_sibling_containers: true' "$tmpdir/linux-config.yaml" >/dev/null
rg -F 'default_isolation_type: docker' "$tmpdir/linux-config.yaml" >/dev/null
rg -F 'max_runner_memory_usage_bytes: 4000000000' "$tmpdir/linux-config.yaml" >/dev/null
rg -F 'max_total_memory_usage_bytes: 12000000000' "$tmpdir/linux-config.yaml" >/dev/null
rg -F 'EnvironmentFile=/etc/buildbuddy-release/executor.env' "$tmpdir/linux.service" >/dev/null
rg -F 'ExecStart=/usr/local/bin/buildbuddy-executor --config_file=/etc/buildbuddy-release/config.yaml' "$tmpdir/linux.service" >/dev/null
rg -F 'WorkingDirectory=/var/lib/buildbuddy-release' "$tmpdir/linux.service" >/dev/null

rg -F 'root_directory: "/Users/example-user/buildbuddy/remotebuilds"' "$tmpdir/mac-config.yaml" >/dev/null
rg -F 'local_cache_directory: "/Users/example-user/buildbuddy/filecache"' "$tmpdir/mac-config.yaml" >/dev/null
rg -F 'source "/Users/example-user/buildbuddy/executor.env"; exec "/Users/example-user/buildbuddy/bin/buildbuddy-executor" --config_file "/Users/example-user/buildbuddy/config.yaml"' "$tmpdir/mac.plist" >/dev/null

echo "buildbuddy_bootstrap_templates_contract: OK"
