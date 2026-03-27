# igdl

`igdl` is a small CLI for downloading Instagram video posts and reels with cookies from a browser you are already logged into.

## Install in one command

macOS and Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/zouyonghe/igdl/main/scripts/install.sh | bash
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/zouyonghe/igdl/main/scripts/install.ps1 | iex
```

Both installers fetch the latest GitHub Release for your platform.

## Quick start

```bash
igdl "https://www.instagram.com/reel/abc123/"
```

More examples:

```bash
igdl "https://www.instagram.com/p/abc123/"
igdl "https://www.instagram.com/reel/abc123/" --browser chrome
igdl "https://www.instagram.com/reel/abc123/" --output "$HOME/Desktop/instagram"
igdl "https://www.instagram.com/reel/abc123/" --verbose
```

Useful flags:

- `--browser <name>` use `chrome`, `edge`, `brave`, `firefox`, or `safari`
- `--output <dir>` save to a different folder
- `--verbose` print browser retry progress

## Default download locations

If you do not pass `--output`, `igdl` uses:

- macOS: `~/Movies/instagram`; if `~/Movies` is missing but `~/Videos` exists, `~/Videos/instagram`
- Linux: your native videos directory with `instagram` appended; usually `$XDG_VIDEOS_DIR/instagram` or `~/Videos/instagram`
- Windows: your Videos folder with `instagram` appended, usually `%USERPROFILE%\Videos\instagram`

The final `instagram` folder is created automatically.

## Supported platforms

Release binaries are available for:

- macOS Apple Silicon (`aarch64`)
- macOS Intel (`x86_64`)
- Linux (`x86_64`)
- Windows (`x86_64`)

## Browser and cookie notes

- Supported browsers: Chrome, Edge, Brave, Firefox, Safari
- By default, `igdl` tries browsers in this order: Chrome, Edge, Brave, Firefox, Safari
- You must already be logged into Instagram in at least one supported browser
- Supported Instagram video URL types include `reel`, `reels`, `p`, and `tv`

## Troubleshooting

- `browser cookies unavailable`: open Instagram in a supported browser, confirm you are logged in, then try again
- If `igdl` is not found after install, restart your terminal or add the install directory to your `PATH`
- Default install location on macOS and Linux: `~/.local/bin/igdl`
- Default install location on Windows: `%LOCALAPPDATA%\Programs\igdl\igdl.exe`

## Development

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test
```

Pushing a tag like `vX.Y.Z` publishes fresh release binaries to GitHub Releases.
