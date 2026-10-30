#!/usr/bin/env bash
# Deploy the service.
set -euo pipefail

source ./lib/common.sh
. "$HOME/.profile"

TARGET="${DEPLOY_TARGET:-staging}"

build() {
  local out="dist"
  cargo build --release
  echo "$out $TARGET"
}

deploy() {
  build
  rsync -a dist/ "$REMOTE_HOST:/srv"
  notify "$TARGET"
}

deploy
