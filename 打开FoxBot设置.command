#!/bin/sh
# Development checkout launcher; no Keychain setup and no implicit model calls.
set -eu
cd "$(dirname "$0")"
if [ ! -x target/debug/foxbot-host ]; then
  printf '%s\n' '请先在项目目录运行 cargo build -p foxbot-host，之后双击此文件即可打开设置。'
  read -r ignored
  exit 1
fi
exec target/debug/foxbot-host settings
