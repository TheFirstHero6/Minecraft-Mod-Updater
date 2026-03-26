# mod-updater

A Rust TUI for checking and updating Minecraft mods from official host metadata, with a focus on Modrinth and optional CurseForge support.

`mod-updater` runs on Linux, macOS, and Windows.

## Features

- Scans a configured Minecraft `mods` directory for `.jar` files
- Resolves updates against Modrinth using official API metadata
- Optionally uses CurseForge when an API key is configured
- Uses strict Minecraft-version matching logic
- Verifies downloaded files before keeping them
- Provides a keyboard-driven TUI for reviewing and applying updates

## Requirements

- Network access
- One of:
  - A release binary from GitHub Releases, or
  - Rust toolchain (`cargo`, `rustc`) if installing from source

Install Rust with [rustup](https://rustup.rs/) if needed.

## Install

### Option A: Prebuilt release binaries (recommended)

Download the latest archive for your platform from GitHub Releases:

- Linux: `mod-updater-x86_64-unknown-linux-gnu.tar.gz`
- macOS: `mod-updater-x86_64-apple-darwin.tar.gz`
- Windows: `mod-updater-x86_64-pc-windows-msvc.zip`

Each release includes a matching `.sha256` file for checksum validation.

#### Linux / macOS quick steps

```bash
tar -xzf mod-updater-<target>.tar.gz
chmod +x mod-updater-<target>/mod-updater
./mod-updater-<target>/mod-updater
```

Optional: move the binary into a folder already on your `PATH`.

#### Windows quick steps

1. Extract `mod-updater-x86_64-pc-windows-msvc.zip`
2. Run `mod-updater.exe` from the extracted folder
3. Optional: move the executable into a folder on `%PATH%`

PowerShell checksum example:

```powershell
Get-FileHash .\mod-updater-x86_64-pc-windows-msvc.zip -Algorithm SHA256
```

### Option B: cargo install fallback

Clone and install from source:

```bash
git clone <your-repo-url>
cd mod-updater
cargo install --path .
```

Then run:

```bash
mod-updater
```

### Option C: Build without installing

```bash
git clone <your-repo-url>
cd mod-updater
cargo build --release
```

Binary output:

- Linux/macOS: `target/release/mod-updater`
- Windows: `target/release/mod-updater.exe`

## Configuration

By default, the app looks for `mod-updater/config.toml` in your platform config directory:

- Linux: `~/.config/mod-updater/config.toml`
- macOS: `~/Library/Application Support/mod-updater/config.toml` (platform-resolved)
- Windows: `%APPDATA%\\mod-updater\\config.toml` (platform-resolved)

Example config:

```toml
mods_dir = "/absolute/path/to/your/minecraft/mods"
minecraft_version = "1.21.1"
loaders = ["fabric"]
user_agent = "yourname/mod-updater/0.1.0 (you@example.com)"

# Optional
curseforge_api_key = ""
concurrency = 8

[download]
backup = true
dry_run = false
verify_after_download = true
```

### Important config fields

- `mods_dir`
  - Required
  - Must point to the exact `mods` directory for the Minecraft instance you are updating
- `minecraft_version`
  - Defaults to `1.21.1` if omitted
  - Should be set to the exact Minecraft version your instance is running, for example `1.21.11`
- `loaders`
  - Required
  - Valid values include `fabric`, `forge`, `neoforge`, and `quilt`
- `user_agent`
  - Strongly recommended for Modrinth
  - Example: `yourname/mod-updater/0.1.0 (you@example.com)`
- `curseforge_api_key`
  - Optional
  - Only needed if you want CurseForge fallback support
- `[download].verify_after_download`
  - Recommended to keep enabled
  - Rejects incompatible downloads and restores from backup when possible

## Environment Variables

These can be used in addition to config:

- `MOD_UPDATER_MODS_DIR` (fallback for `mods_dir`)
- `CURSEFORGE_API_KEY` (fallback for `curseforge_api_key`)
- `MOD_UPDATER_ASCII` (optional; force ASCII spinner/icons in TUI)

## CLI Usage

Basic run:

```bash
mod-updater
```

Override config values from the command line:

```bash
mod-updater \
  --mods-dir /path/to/mods \
  --minecraft-version 1.21.11 \
  --loaders fabric \
  --user-agent "yourname/mod-updater/0.1.0 (you@example.com)"
```

Available CLI flags:

- `--config`
- `--mods-dir`
- `--minecraft-version`
- `--loaders`
- `--user-agent`
- `--curseforge-api-key`

## TUI Controls

- `j` / `Down`: move selection down
- `k` / `Up`: move selection up
- `d`: download update for selected row
- `r`: refresh and rescan
- `?`: toggle help
- `q` / `Esc`: quit

## Notes On Accuracy

For best results:

- Set `minecraft_version` to the exact version your instance runs
- Set the correct loader in `loaders`
- Keep `verify_after_download = true`
- Use a proper `user_agent` so Modrinth requests are well-formed

## Development

Run the app in debug:

```bash
cargo run
```

Run tests:

```bash
cargo test
```

Run lint checks:

```bash
cargo clippy --all-targets
```
