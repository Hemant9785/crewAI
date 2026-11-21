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
install -m 0755 "${TEMP_DIR}/unpacked/herdr" "${INSTALL_DIR}/herdr"
install -m 0644 "${TEMP_DIR}/unpacked/LICENSE" "${HOME}/.local/share/licenses/team-herdr/LICENSE"

add_path_line() {
    rc_file="$1"
    path_line="case \":\$PATH:\" in *\":$INSTALL_DIR:\"*) ;; *) export PATH=\"$INSTALL_DIR:\$PATH\" ;; esac"
    if ! grep -Fqx "$path_line" "$rc_file" 2>/dev/null; then
        printf '\n# Added by the Team Herdr installer\n%s\n' "$path_line" >> "$rc_file"
    fi
}

case "${SHELL##*/}" in
    bash)
        add_path_line "$HOME/.bashrc"
        if [ -f "$HOME/.bash_profile" ]; then
            add_path_line "$HOME/.bash_profile"
        else
            add_path_line "$HOME/.profile"
        fi
        ;;
    zsh)
        add_path_line "$HOME/.zshrc"
        if [ -f "$HOME/.zprofile" ]; then
            add_path_line "$HOME/.zprofile"
        else
            add_path_line "$HOME/.profile"
        fi
        ;;
    fish)
        fish_config="$HOME/.config/fish/conf.d"
        mkdir -p "$fish_config"
        fish_path_file="$fish_config/team-herdr-path.fish"
        fish_path_line="fish_add_path \"$INSTALL_DIR\""
        if ! grep -Fqx "$fish_path_line" "$fish_path_file" 2>/dev/null; then
            printf '%s\n' "$fish_path_line" >> "$fish_path_file"
        fi
        ;;
    *)
        add_path_line "$HOME/.profile"
        ;;
esac

printf 'Installed herdr to %s/herdr\n' "$INSTALL_DIR"
printf 'Open a new terminal and run: herdr\n'
