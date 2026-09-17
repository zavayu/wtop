# wtop

`wtop` is a read-only, htop-style system monitor for Windows terminals. It
shows live CPU, memory, Windows commit, and process information in a full-screen
terminal UI.

## Current features

- A refreshable process table with PID, name, CPU usage, memory use, and command
  line.
- A system header with a total-CPU history view, physical memory used/total
  bar, and Windows commit charge/limit bar. Bars hold the last read value when
  a query fails. CPU values appear after the first refresh interval, once a
  sampling baseline exists.
- Toggle from the compact total-CPU history to a responsive grid of current
  logical-CPU meters; the header grows to accommodate the grid.
- Selection, vertical process-list scrolling, and command-line scrolling that
  preserves Unicode display-cell boundaries.
- Flat and parent/child tree process views, with expandable process groups.
- Process sorting and live case-insensitive filtering; tree filters retain the
  ancestors of matching processes for context.
- Safe resize behavior, a 60 columns by 12 rows minimum-size message, and
  terminal restoration on exit.

## Requirements

- Windows with PowerShell or Windows Terminal.
- The current stable Rust toolchain, installed through
  [rustup](https://rustup.rs/).

## Run during development

```powershell
cargo run
```

## Install the `wtop` command locally

```powershell
cargo install --path .
```

Ensure Cargo's bin directory is on your `PATH`, then run:

```powershell
wtop
```

## Keybindings

| Key | Action |
| --- | --- |
| `Up` / `Down` | Move the selected process |
| `Page Up` / `Page Down` | Move by one visible page |
| `Home` / `End` | Select the first / last visible process |
| `Left` / `Right` | Scroll the selected command line |
| `c` | Toggle total CPU history and logical-CPU meters |
| `t` | Toggle flat and tree process views |
| `Enter` or `Space` | Expand or collapse the selected tree process |
| `s` | Cycle PID, name, CPU, and memory sorting |
| `S` | Reverse the active sort direction |
| `/` | Edit a filter; it applies as you type |
| `Enter` | Keep the edited filter |
| `Esc` | Cancel filter editing, or quit when not editing |
| `Ctrl+U` | Clear the filter while editing |
| `q` or `Ctrl+C` | Quit |

Filtering matches process names and command lines that Windows allowed wtop to
read. The table title shows the active sort and current filter.

Process CPU is normalized to total machine capacity, so it ranges from 0 to
100%, matching the CPU value in the system header.

## Terminology and access limitations

`Mem` is physical memory used/total. `Commit` is the Windows commit charge and
commit limit reported by `GetPerformanceInfo`; it is not a Linux-style swap
measurement.

Windows restricts access to some protected and system processes. Their command
line is shown as `<unavailable>` when it cannot be read, with an executable-path
fallback when available. Running from an elevated terminal can reveal more
details, but does not guarantee access to protected processes.

wtop does not alter processes. Termination, priority changes, saved views,
custom columns, and disk/network/GPU metrics are outside this first milestone.

## Validate a build

```powershell
cargo fmt --check
cargo test
cargo clippy -- -D warnings
cargo build --release
```

The release executable is written to `target\release\wtop.exe`.

## Manual verification checklist

Before publishing a build, test it in both a normal and (optionally) elevated
PowerShell or Windows Terminal session:

- Confirm values and the process list refresh without blocking navigation, and
  that the CPU sparkline advances and the memory/commit bars move with the
  readouts.
- Move through a process list taller than the terminal and inspect a long
  command line with `Left` and `Right`.
- Toggle tree mode with `t`, expand/collapse with `Enter` or `Space`, then sort
  with `s`/`S` and filter with `/`; accept, cancel, and clear a filter.
- Resize the terminal, including to 60 columns by 12 rows, then quit with
  `q`, `Esc`, and `Ctrl+C` to confirm terminal restoration.
- Confirm protected processes remain visible even when command-line details are
  unavailable.
