#!/usr/bin/env bash
# Build katok (cli/) and the MCP server (mcp/), then register the server with Claude Code.
#
# Registers at user scope. A first install passes NO environment variables: the archive key lives
# in the macOS Keychain (created by the katok_init tool), never in ~/.claude.json.
#
# Re-running rebuilds and replaces the existing katok-mcp registration, and carries over the
# environment variables of the old one (KATOK_MCP_DATA_DIR, KATOK_MCP_ALLOWED_CHATS,
# KATOK_MCP_OUT_ROOTS, ...), read from `claude mcp get katok-mcp`. The server command is
# registered with the absolute path of the node found on PATH now, so a GUI-launched Claude
# Code does not depend on its own PATH. If `claude mcp add` fails, the script exits non-zero.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found. Install Rust (https://rustup.rs) first." >&2
  exit 127
fi
for tool in node pnpm claude; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "$tool not found on PATH." >&2
    exit 127
  fi
done

echo "==> cargo build --release (cli/)"
(cd "$ROOT/cli" && cargo build --release)
test -x "$ROOT/cli/target/release/katok"
# Build stamp: the commit this binary was built from. katok_doctor reports it and warns when the
# stamp is missing or the binary is older than cli/src (e.g. a branch switch without a rebuild).
BUILD_REF="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
printf '%s\n' "$BUILD_REF" >"$ROOT/cli/target/release/katok.ref"
echo "    build stamp: $BUILD_REF"

echo "==> pnpm install + build (mcp/)"
(cd "$ROOT/mcp" && pnpm install --frozen-lockfile && pnpm build)
test -f "$ROOT/mcp/dist/index.js"

echo "==> claude mcp add katok-mcp (user scope)"
NODE_BIN="$(command -v node)"

# Keep the old registration's environment: `claude mcp get` prints it as an "Environment:" block
# of indented KEY=VALUE lines. Values may contain "=" and spaces.
ENV_ARGS=()
ENV_NAMES=()
if existing="$(claude mcp get katok-mcp 2>/dev/null)"; then
  in_env=0
  while IFS= read -r line; do
    if [[ $in_env -eq 0 ]]; then
      if [[ "$line" =~ ^[[:space:]]*Environment:[[:space:]]*$ ]]; then in_env=1; fi
      continue
    fi
    if [[ "$line" =~ ^[[:space:]]+([A-Za-z_][A-Za-z0-9_]*)=(.*)$ ]]; then
      ENV_ARGS+=(-e "${BASH_REMATCH[1]}=${BASH_REMATCH[2]}")
      ENV_NAMES+=("${BASH_REMATCH[1]}")
    else
      break
    fi
  done <<<"$existing"
fi
if [[ ${#ENV_NAMES[@]} -gt 0 ]]; then
  echo "    keeping environment variables: ${ENV_NAMES[*]}"
fi

claude mcp remove katok-mcp -s user >/dev/null 2>&1 || true
if ! claude mcp add --scope user katok-mcp ${ENV_ARGS[@]+"${ENV_ARGS[@]}"} -- "$NODE_BIN" "$ROOT/mcp/dist/index.js"; then
  {
    echo "ERROR: 'claude mcp add' failed. The old katok-mcp registration was already removed,"
    echo "so Claude Code currently has NO katok-mcp server."
    if [[ ${#ENV_NAMES[@]} -gt 0 ]]; then
      echo "Its environment variables were: ${ENV_NAMES[*]}"
      echo "Re-add them with -e NAME=VALUE (values are not printed here), then run:"
    fi
    echo "  claude mcp add --scope user katok-mcp -- \"$NODE_BIN\" \"$ROOT/mcp/dist/index.js\""
  } >&2
  exit 1
fi

cat <<MSG

Done. Next steps:
  1. Grant Full Disk Access to the app that runs Claude Code (Terminal, iTerm, ...).
     "$ROOT/cli/target/release/katok" permissions macos   opens the settings page.
  2. In Claude Code, call the katok_init tool once: it creates the Keychain key, the data
     folder (~/Library/Application Support/katok-mcp) and runs the first sync.
MSG
