#!/usr/bin/env bash
# Build katok (cli/) and the MCP server (mcp/), then register the server with Claude Code and/or Codex.
# Without cargo (or with KATOK_PREBUILT=1) katok is downloaded from GitHub Releases instead.
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

BIN_DIR="$ROOT/cli/target/release"
RELEASE_REPO="DaunJeong02/katok-mcp"
ASSET="katok-aarch64-apple-darwin.tar.gz"

# Prebuilt katok from GitHub Releases: unsigned (ad-hoc), Apple silicon only, checked against its
# SHA-256. The release for this checkout's tag when HEAD is tagged, otherwise the latest one.
download_katok() {
  if [[ "$(uname -s)-$(uname -m)" != "Darwin-arm64" ]]; then
    echo "Prebuilt katok is for Apple silicon Macs only. Install Rust (https://rustup.rs) to build from source." >&2
    exit 127
  fi
  local tag base tmp
  tag="$(git -C "$ROOT" describe --tags --exact-match 2>/dev/null || true)"
  if [[ -n "$tag" ]]; then
    base="https://github.com/$RELEASE_REPO/releases/download/$tag"
  else
    base="https://github.com/$RELEASE_REPO/releases/latest/download"
  fi
  tmp="$(mktemp -d)"
  echo "==> download katok ($base/$ASSET)"
  curl -fsSL -o "$tmp/$ASSET" "$base/$ASSET"
  curl -fsSL -o "$tmp/$ASSET.sha256" "$base/$ASSET.sha256"
  (cd "$tmp" && shasum -a 256 -c "$ASSET.sha256")
  mkdir -p "$BIN_DIR"
  tar -xzf "$tmp/$ASSET" -C "$BIN_DIR" katok katok.ref
  touch "$BIN_DIR/katok.ref"
  rm -rf "$tmp"
  echo "    build stamp: $(cat "$BIN_DIR/katok.ref")"
}

USE_PREBUILT="${KATOK_PREBUILT:-0}"
if [[ "$USE_PREBUILT" != 1 ]] && ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found, using the prebuilt katok."
  USE_PREBUILT=1
fi
for tool in node pnpm; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "$tool not found on PATH." >&2
    exit 127
  fi
done
if ! command -v claude >/dev/null 2>&1 && ! command -v codex >/dev/null 2>&1; then
  echo "Neither claude nor codex found on PATH." >&2
  exit 127
fi

if [[ "$USE_PREBUILT" == 1 ]]; then
  download_katok
else
  echo "==> cargo build --release (cli/)"
  (cd "$ROOT/cli" && cargo build --release)
  # Build stamp: the commit this binary was built from. katok_doctor reports it and warns when the
  # stamp is missing or the binary is older than cli/src (e.g. a branch switch without a rebuild).
  BUILD_REF="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
  printf '%s\n' "$BUILD_REF" >"$BIN_DIR/katok.ref"
  echo "    build stamp: $BUILD_REF"
fi
test -x "$BIN_DIR/katok"

echo "==> pnpm install + build (mcp/)"
(cd "$ROOT/mcp" && pnpm install --frozen-lockfile && pnpm build)
test -f "$ROOT/mcp/dist/index.js"

NODE_BIN="$(command -v node)"
SERVER="$ROOT/mcp/dist/index.js"

if command -v claude >/dev/null 2>&1; then
  echo "==> claude mcp add katok-mcp (user scope)"

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
  if ! claude mcp add --scope user katok-mcp ${ENV_ARGS[@]+"${ENV_ARGS[@]}"} -- "$NODE_BIN" "$SERVER"; then
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
fi

# Codex: add once, then leave it alone. `codex mcp get` masks env values, so re-adding would drop
# them. Codex's default tool timeout (60 s) is shorter than init/sync/backfill, so raise it.
if command -v codex >/dev/null 2>&1; then
  echo "==> codex mcp add katok-mcp"
  if codex mcp get katok-mcp >/dev/null 2>&1; then
    echo "    already registered, keeping it"
  else
    codex mcp add katok-mcp -- "$NODE_BIN" "$SERVER"
  fi
  CODEX_CONFIG="${CODEX_HOME:-$HOME/.codex}/config.toml"
  if [[ -f "$CODEX_CONFIG" ]] && ! awk '/^\[/{s=($0=="[mcp_servers.katok-mcp]")} s&&/^tool_timeout_sec/{f=1} END{exit !f}' "$CODEX_CONFIG"; then
    tmp="$(mktemp)"
    awk '{print} $0=="[mcp_servers.katok-mcp]"{print "tool_timeout_sec = 1800"}' "$CODEX_CONFIG" >"$tmp"
    cat "$tmp" >"$CODEX_CONFIG" && rm -f "$tmp"
    echo "    tool_timeout_sec = 1800"
  fi
fi

cat <<MSG

Done. Next steps:
  1. Grant Full Disk Access to the app that runs Claude Code / Codex (Terminal, iTerm, Codex, ...).
     "$ROOT/cli/target/release/katok" permissions macos   opens the settings page.
  2. Restart Claude Code / Codex and call the katok_init tool once: it creates the Keychain key, the data
     folder (~/Library/Application Support/katok-mcp) and runs the first sync.
MSG
