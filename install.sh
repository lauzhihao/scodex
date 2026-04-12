#!/usr/bin/env bash
set -euo pipefail

RAW_BASE="${AUTO_CODEX_RAW_BASE:-https://raw.githubusercontent.com/lauzhihao/auto-codex/main}"
INSTALL_BIN="${HOME}/.local/bin"
INSTALL_HOME="${HOME}/.local/share/auto-codex"
SCRIPT_URL="${RAW_BASE}/codex-autoswitch.py"
SCRIPT_PATH="${INSTALL_HOME}/codex-autoswitch.py"
WRAPPER_PATH="${INSTALL_BIN}/auto-codex"

if [[ "${RAW_BASE}" == *"<your-name>"* || "${RAW_BASE}" == *"<repo>"* ]]; then
  echo "Replace the placeholder GitHub repo in install.sh before publishing, or set AUTO_CODEX_RAW_BASE." >&2
  exit 1
fi

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

need_cmd bash
need_cmd curl
need_cmd python3
need_cmd codex

mkdir -p "${INSTALL_BIN}" "${INSTALL_HOME}"

tmp_script="$(mktemp "${TMPDIR:-/tmp}/auto-codex.XXXXXX.py")"
trap 'rm -f "${tmp_script}"' EXIT

curl -fsSL "${SCRIPT_URL}" -o "${tmp_script}"
install -m 0755 "${tmp_script}" "${SCRIPT_PATH}"

cat > "${WRAPPER_PATH}" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
export AUTO_CODEX_HOME="${HOME}/.local/share/auto-codex"
export AUTO_CODEX_PROG="auto-codex"
exec python3 "${AUTO_CODEX_HOME}/codex-autoswitch.py" "$@"
EOF

chmod 0755 "${WRAPPER_PATH}"

case ":${PATH}:" in
  *":${HOME}/.local/bin:"*) ;;
  *)
    echo "Installed to ${WRAPPER_PATH}"
    echo "Add ${HOME}/.local/bin to PATH before using auto-codex."
    exit 0
    ;;
esac

echo "Installed to ${WRAPPER_PATH}"
echo "Run: auto-codex list"
