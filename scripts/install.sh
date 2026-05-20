#!/usr/bin/env sh
set -eu

PREFIX="${1:-$HOME/.local}"
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
RUMB_HOME=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
BIN_DIR="$PREFIX/bin"

cargo build --release --manifest-path "$RUMB_HOME/Cargo.toml" --bin rumb --bin rumb-mcp >/dev/null
mkdir -p "$BIN_DIR"

cat > "$BIN_DIR/rumb" <<EOF
#!/usr/bin/env sh
set -eu
export RUMB_HOME="$RUMB_HOME"
export RUMB_SHIM="$BIN_DIR/rumb"
exec "\$RUMB_HOME/target/release/rumb" "\$@"
EOF

cat > "$BIN_DIR/rumb-mcp" <<EOF
#!/usr/bin/env sh
set -eu
export RUMB_HOME="$RUMB_HOME"
export RUMB_MCP_SHIM="$BIN_DIR/rumb-mcp"
exec "\$RUMB_HOME/target/release/rumb-mcp" "\$@"
EOF

chmod +x "$BIN_DIR/rumb" "$BIN_DIR/rumb-mcp"

printf 'installed rumb to %s\n' "$BIN_DIR/rumb"
printf 'installed rumb-mcp to %s\n' "$BIN_DIR/rumb-mcp"
printf 'ensure %s is on PATH\n' "$BIN_DIR"
