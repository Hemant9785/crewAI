#!/bin/sh
set -eu

REPOSITORY="Hemant9785/crewAI"
RELEASE_URL="https://github.com/${REPOSITORY}/releases/latest/download"
INSTALL_DIR="${TEAM_HERDR_INSTALL_DIR:-$HOME/.local/bin}"

fail() {
    printf 'team-herdr installer: %s\n' "$1" >&2
    exit 1
}

command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v tar >/dev/null 2>&1 || fail "tar is required"

case "$(uname -s)" in
    Linux) ;;
    *) fail "this installer supports Linux; download the matching release asset for other systems" ;;
esac

case "$(uname -m)" in
    x86_64|amd64) ASSET="team-herdr-linux-x86_64.tar.gz" ;;
    *) fail "the current release supports Linux x86_64; this machine reports $(uname -m)" ;;
esac

TEMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TEMP_DIR"' EXIT HUP INT TERM

curl -fsSL --retry 3 --connect-timeout 10 --max-time 120 "${RELEASE_URL}/${ASSET}" -o "${TEMP_DIR}/${ASSET}" \
    || fail "could not download ${ASSET}; check that a project release has been published"
curl -fsSL --retry 3 --connect-timeout 10 --max-time 30 "${RELEASE_URL}/SHA256SUMS" -o "${TEMP_DIR}/SHA256SUMS" \
    || fail "could not download release checksums"

EXPECTED="$(awk -v asset="$ASSET" '$2 == asset { print $1; exit }' "${TEMP_DIR}/SHA256SUMS")"
[ "${#EXPECTED}" -eq 64 ] || fail "release checksum entry is missing or malformed"

if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL="$(sha256sum "${TEMP_DIR}/${ASSET}" | awk '{ print $1 }')"
elif command -v shasum >/dev/null 2>&1; then
    ACTUAL="$(shasum -a 256 "${TEMP_DIR}/${ASSET}" | awk '{ print $1 }')"
else
    fail "sha256sum or shasum is required to verify the download"
fi
[ "$ACTUAL" = "$EXPECTED" ] || fail "download checksum does not match the release"

mkdir -p "$TEMP_DIR/unpacked" "$INSTALL_DIR" "${HOME}/.local/share/licenses/team-herdr"
tar -xzf "${TEMP_DIR}/${ASSET}" -C "$TEMP_DIR/unpacked"
[ -x "${TEMP_DIR}/unpacked/herdr" ] || fail "release archive does not contain the executable"
[ -f "${TEMP_DIR}/unpacked/LICENSE" ] || fail "release archive does not contain the license"
install -m 0755 "${TEMP_DIR}/unpacked/herdr" "${INSTALL_DIR}/team-herdr"
install -m 0644 "${TEMP_DIR}/unpacked/LICENSE" "${HOME}/.local/share/licenses/team-herdr/LICENSE"

printf 'Installed team-herdr to %s/team-herdr\n' "$INSTALL_DIR"
printf 'Launch it with: %s/team-herdr\n' "$INSTALL_DIR"
