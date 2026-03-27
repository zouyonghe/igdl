#!/usr/bin/env bash

set -euo pipefail

REPO="zouyonghe/igdl"
RELEASES_URL="https://github.com/${REPO}/releases/latest"
INSTALL_DIR="${IGDL_INSTALL_DIR:-$HOME/.local/bin}"
TMP_DIR=""

say() {
  printf '%s\n' "$*"
}

die() {
  printf 'Error: %s\n' "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

checksum_command() {
  if command -v shasum >/dev/null 2>&1; then
    printf '%s\n' 'shasum'
    return
  fi

  if command -v sha256sum >/dev/null 2>&1; then
    printf '%s\n' 'sha256sum'
    return
  fi

  die "missing required command: shasum or sha256sum"
}

cleanup() {
  if [ -n "$TMP_DIR" ]; then
    rm -rf "$TMP_DIR"
  fi
}

detect_archive_suffix() {
  local os arch

  os=$(uname -s)
  arch=$(uname -m)

  case "$os" in
    Darwin)
      case "$arch" in
        arm64|aarch64)
          printf '%s\n' 'macos-aarch64.tar.gz'
          ;;
        x86_64)
          printf '%s\n' 'macos-x86_64.tar.gz'
          ;;
        *)
          die "unsupported macOS architecture: $arch"
          ;;
      esac
      ;;
    Linux)
      case "$arch" in
        x86_64|amd64)
          printf '%s\n' 'linux-x86_64.tar.gz'
          ;;
        *)
          die "unsupported Linux architecture: $arch"
          ;;
      esac
      ;;
    *)
      die "unsupported operating system: $os"
      ;;
  esac
}

resolve_latest_tag() {
  local release_url

  release_url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$RELEASES_URL")
  release_url=${release_url%/}

  printf '%s\n' "${release_url##*/}"
}

sha256_digest() {
  local tool file_path digest _rest

  tool=$(checksum_command)
  file_path=$1

  case "$tool" in
    shasum)
      read -r digest _rest < <(shasum -a 256 "$file_path")
      ;;
    sha256sum)
      read -r digest _rest < <(sha256sum "$file_path")
      ;;
  esac

  printf '%s\n' "$digest"
}

expected_checksum() {
  local checksum_file archive_name digest file_name remainder

  checksum_file=$1
  archive_name=$2

  while read -r digest file_name remainder; do
    if [ "$file_name" = "$archive_name" ]; then
      printf '%s\n' "$digest"
      return
    fi
  done < "$checksum_file"

  die "missing checksum for ${archive_name}"
}

verify_archive_checksum() {
  local tag archive_name archive_path checksum_url checksum_path expected actual

  tag=$1
  archive_name=$2
  archive_path=$3
  checksum_url="https://github.com/${REPO}/releases/download/${tag}/SHA256SUMS.txt"
  checksum_path="$TMP_DIR/SHA256SUMS.txt"

  say "Downloading SHA256SUMS.txt..."
  curl -fsSL "$checksum_url" -o "$checksum_path"

  expected=$(expected_checksum "$checksum_path" "$archive_name")
  actual=$(sha256_digest "$archive_path")

  [ "$expected" = "$actual" ] || die "checksum verification failed for ${archive_name}"

  say "Verified checksum for ${archive_name}."
}

main() {
  local suffix tag archive_name download_url archive_path binary_path

  require_command curl
  require_command tar
  require_command install

  suffix=$(detect_archive_suffix)
  tag=$(resolve_latest_tag)
  archive_name="igdl-${tag}-${suffix}"
  download_url="https://github.com/${REPO}/releases/download/${tag}/${archive_name}"

  TMP_DIR=$(mktemp -d)

  mkdir -p "$INSTALL_DIR"

  archive_path="$TMP_DIR/$archive_name"
  binary_path="$TMP_DIR/igdl"

  say "Downloading ${archive_name}..."
  curl -fsSL "$download_url" -o "$archive_path"
  verify_archive_checksum "$tag" "$archive_name" "$archive_path"

  tar -xzf "$archive_path" -C "$TMP_DIR"
  install -m 0755 "$binary_path" "$INSTALL_DIR/igdl"

  say "Installed igdl ${tag} to $INSTALL_DIR/igdl"
  say "Usage: igdl <instagram-url>"
  say "Example: igdl \"https://www.instagram.com/reel/abc123/\" --browser chrome"

  case ":$PATH:" in
    *":$INSTALL_DIR:"*)
      ;;
    *)
      say "Add $INSTALL_DIR to your PATH if needed:"
      say "  export PATH=\"$INSTALL_DIR:\$PATH\""
      ;;
  esac
}

trap cleanup EXIT
main "$@"
