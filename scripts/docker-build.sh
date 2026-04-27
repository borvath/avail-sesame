#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SESAME_ROOT="$(cd "${APP_ROOT}/../Sesame" && pwd)"
SCRUTINIZER_ROOT="$(cd "${APP_ROOT}/../scrutinizer" && pwd)"
TMP_ROOT="$(mktemp -d)"

IMAGE_TAG="${1:-avail:latest}"
shift || true

cleanup() {
    rm -rf "${TMP_ROOT}"
}

sync_context() {
    local source_dir="$1"
    local target_dir="$2"

    mkdir -p "${target_dir}"
    rsync -a \
        --delete \
        --exclude '.git/' \
        --exclude '.idea/' \
        --exclude 'target/' \
        --exclude 'scrutinizer.log' \
        --exclude '*.result.json' \
        "${source_dir}/" "${target_dir}/"
}

trap cleanup EXIT

sync_context "${SESAME_ROOT}" "${TMP_ROOT}/sesame"
sync_context "${SCRUTINIZER_ROOT}" "${TMP_ROOT}/scrutinizer"

docker buildx build \
    --load \
    --build-context sesame="${TMP_ROOT}/sesame" \
    --build-context scrutinizer="${TMP_ROOT}/scrutinizer" \
    -t "${IMAGE_TAG}" \
    "$@" \
    "${APP_ROOT}"
