#!/usr/bin/env sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
RUMB_HOME=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

cargo build --manifest-path "$RUMB_HOME/Cargo.toml" --bin rumb --bin rumb-mcp "$@"
