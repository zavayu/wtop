# wtop

> An htop-inspired system monitor for Windows terminals.

`wtop` provides a fast, full-screen view of running processes and system
resource usage from PowerShell or Windows Terminal. Navigate the process list,
inspect parent/child relationships, filter and sort live data, and manage a
selected process without leaving the command line.

## Highlights

- Live CPU, memory, Windows commit, network, GPU, and process monitoring.
- Sortable and filterable process table with PID, user, thread count, CPU, and
  memory usage.
- Flat and collapsible tree views for process hierarchies.
- Logical CPU meters and aggregate CPU history views.
- Per-adapter GPU overview and detail panes when Windows exposes the counters.
- Eleven built-in color themes, with live preview from the terminal UI.
- Careful terminal cleanup, resize handling, and guarded process termination.

## Quick start

### Prerequisites

- Windows with PowerShell or Windows Terminal.
- The current stable Rust toolchain from [rustup](https://rustup.rs/).

### Run from a checkout

```powershell
cargo run
```

### Install locally

```powershell
cargo install --path .
wtop
```

Ensure Cargo's bin directory is available on your `PATH` before running the
installed command.

## Controls

| Key | Action |
| --- | --- |
| `Up` / `Down` | Move the selected process |
| `Ctrl+Up` / `Ctrl+Down` | Move by one visible page |
| `Ctrl+Left` / `Ctrl+Right` | Select the first / last visible process |
| `c` | Switch between logical CPU meters and total CPU history |
| `g` | Cycle GPU overview and individual adapter panes |
| `t` | Toggle flat and tree process views |
| `Enter` or `Space` | Expand or collapse the selected node in tree view |
| `o` | Open Themes; use `Up`/`Down` to preview, `Enter` to apply, or `Esc` to cancel |
| `?` or `h` | Open Help; dismiss with `?`, `h`, `Enter`, or `Esc` |
| `s` / `S` | Cycle the sort column / reverse its direction |
| `/` | Edit a live filter (`Ctrl+U` clears it) |
| `x` | Request termination of the selected process; confirm with `y` |
| `q` or `Ctrl+C` | Quit |

The Themes menu includes Default, Monochromatic, Black on White, Light
Terminal, MC, Black Night, Broken Gray, Nord, Ocean, Evergreen, and Dusk.
Theme selection applies to the current session.

## Notes

`Mem` is physical memory used versus total memory. `Commit` is Windows commit
charge versus its limit, rather than a Linux-style swap measurement.

Windows protects some system and elevated processes. Command lines, owner
information, GPU counters, and termination permissions are therefore
best-effort; unavailable values are shown as unavailable rather than zero.
`wtop` always asks for confirmation before requesting termination and refuses
to target PID 0, PID 4, or itself.

## Development

Run the standard checks before contributing:

```powershell
cargo fmt --check
cargo test
cargo clippy -- -D warnings
cargo build --release
```

The release executable is written to `target\release\wtop.exe`.
