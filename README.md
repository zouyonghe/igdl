# igdl

`igdl` is a standalone Rust CLI for downloading Instagram videos with browser cookies and `yt-dlp`.

## Features

- Downloads Instagram `reel` and `post` video links
- Reuses logged-in browser cookies
- Defaults to `~/Videos/instagram`
- Falls back to `~/Movies/instagram`
- Bootstraps a managed `yt-dlp` binary when needed

## Installation

Download the latest binary from GitHub Releases and unpack it. The file name below is an example:

```bash
tar -xzf igdl-v0.1.0-macos-aarch64.tar.gz
chmod +x igdl
mv igdl /usr/local/bin/igdl
```

## Usage

```bash
igdl <instagram-url>
```

Examples:

```bash
igdl "https://www.instagram.com/reel/abc123/"
igdl "https://www.instagram.com/p/abc123/" --browser chrome
igdl "https://www.instagram.com/reel/abc123/" --output "$HOME/Desktop/instagram"
igdl "https://www.instagram.com/reel/abc123/" --verbose
```

## Options

- `--browser <name>`: choose `chrome`, `edge`, `brave`, `firefox`, or `safari`
- `--output <dir>`: override the default output directory
- `--verbose`: print browser attempt progress

## Default Output Directory

If `--output` is not provided, `igdl` uses:

1. `~/Videos/instagram` if `~/Videos` exists
2. `~/Movies/instagram` if `~/Videos` does not exist and `~/Movies` does
3. `~/Videos/instagram` otherwise

The final `instagram/` directory is created automatically.

## Browser Cookies

`igdl` tries browser cookies in this order by default:

1. Chrome
2. Edge
3. Brave
4. Firefox
5. Safari

You must already be logged into Instagram in at least one supported browser.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Releases

Pushing a version tag such as `v0.1.0` triggers GitHub Actions to build macOS release binaries and publish them to GitHub Releases.
