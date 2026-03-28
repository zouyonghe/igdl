# igdl

`igdl` downloads Instagram image posts, video posts, and full carousel posts by reusing cookies from a browser where you're already signed in.

## Install in one command

macOS and Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/zouyonghe/igdl/main/scripts/install.sh | bash
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/zouyonghe/igdl/main/scripts/install.ps1 | iex
```

Both installers pull the latest GitHub Release for your platform.

On macOS and Linux, image and carousel posts may create a local `gallery-dl` runtime the first time you use them, so keep `python3` available. If a post contains video and `igdl` says `yt-dlp` is missing, install `yt-dlp` and try again.

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

For `p` posts, `igdl` downloads every item in the post by default. Single-image posts, single-video posts, and mixed-media carousels are all saved in order.

Useful flags:

- `--browser <name>` use `chrome`, `edge`, `brave`, `firefox`, or `safari`
- `--output <dir>` save to a different folder
- `--verbose` show browser retries and backend details when available

By default, `igdl` prints concise progress while it downloads. Reel downloads now show live video progress, including a dynamic progress bar with percent, speed, and ETA when running in a terminal, and carousel posts still show overall progress like `1/3`, `2/3`, and `3/3` as each item finishes.

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
- Supported Instagram URL types include `reel`, `reels`, `p`, and `tv`, covering image posts, video posts, and mixed-media carousel posts

## Troubleshooting

- `browser cookies unavailable`: open Instagram in a supported browser, confirm you are logged in, then try again
- On macOS and Linux, make sure `python3` is available if `igdl` needs to set up its local media-downloader runtime
- If a post contains video and `igdl` says `yt-dlp` is missing, install `yt-dlp` and try again
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
