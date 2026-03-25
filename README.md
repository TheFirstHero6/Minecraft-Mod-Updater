# mod-updater

A Rust TUI for checking and updating Minecraft mods from official host metadata, with a focus on Modrinth and optional CurseForge support.

This project is currently intended for **Unix-like operating systems only** (Linux, macOS, BSD, etc.). It has not been packaged for the AUR yet, so the current installation method is **build from source**.

## Features

- Scans a configured Minecraft `mods` directory for `.jar` files
- Resolves updates against **Modrinth** using official API metadata
- Optionally uses **CurseForge** when an API key is configured
- Uses strict Minecraft-version matching logic
- Verifies downloaded files before keeping them
- Provides a keyboard-driven TUI for reviewing and applying updates

## Requirements

- A Unix-like OS
- Rust toolchain (`cargo`, `rustc`)
- Network access

Install Rust with [rustup](https://rustup.rs/) if needed.

## Build From Source

Clone the repository and build a release binary:

```bash
git clone <your-repo-url>
cd mod-updater
cargo build --release
```

The executable will be created at:

```text
target/release/mod-updater
```

## Install Locally

You can run the binary directly:

```bash
./target/release/mod-updater
```

Or copy/symlink it into `~/.local/bin`:

```bash
mkdir -p ~/.local/bin
ln -sf "$PWD/target/release/mod-updater" ~/.local/bin/mdate
```

If `~/.local/bin` is already on your `PATH`, you can launch it as:

```bash
mdate
```

If you prefer an alias instead, add something like this to `~/.zshrc`:

```zsh
alias mdate="$HOME/.local/bin/mdate"
```

Then reload your shell:

```zsh
source ~/.zshrc
```

## Configuration

By default, the app looks for:

```text
~/.config/mod-updater/config.toml
```

Create the directory if it does not exist:

```bash
mkdir -p ~/.config/mod-updater
```

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
  - Should be set to the **exact** Minecraft version your instance is running, for example `1.21.11`

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

- `MOD_UPDATER_MODS_DIR`
  - Fallback for `mods_dir`

- `CURSEFORGE_API_KEY`
  - Fallback for `curseforge_api_key`

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
- `r`: refresh/rescan
- `q` / `Esc`: quit

## Notes On Accuracy

For best results:

- Set `minecraft_version` to the **exact** version your instance runs
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
