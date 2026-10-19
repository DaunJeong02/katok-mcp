#!/usr/bin/env bash
# Build katok (cli/) and the MCP server (mcp/), then register the server with Claude Code.
#
# Registers at user scope with NO environment variables: the archive key lives in the macOS
# Keychain (created by the katok_init tool), never in ~/.claude.json.
#
# Re-running is safe: it rebuilds and replaces the existing katok-mcp registration.
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

echo "==> pnpm install + build (mcp/)"
(cd "$ROOT/mcp" && pnpm install --frozen-lockfile && pnpm build)
test -f "$ROOT/mcp/dist/index.js"

echo "==> claude mcp add katok-mcp (user scope)"
claude mcp remove katok-mcp -s user >/dev/null 2>&1 || true
claude mcp add --scope user katok-mcp -- node "$ROOT/mcp/dist/index.js"

cat <<MSG

Done. Next steps:
  1. Grant Full Disk Access to the app that runs Claude Code (Terminal, iTerm, ...).
     "$ROOT/cli/target/release/katok" permissions macos   opens the settings page.
  2. In Claude Code, call the katok_init tool once: it creates the Keychain key, the data
     folder (~/Library/Application Support/katok-mcp) and runs the first sync.
MSG
