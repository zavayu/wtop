# wtop

`wtop` is an htop-style system monitor for Windows terminals. It
shows live CPU, memory, Windows commit, and process information in a full-screen
terminal UI.

## Current features

- A refreshable process table with PID, user, thread count, CPU usage, memory
  use, and a flexible name column that keeps process trees readable.
- A system header with a total-CPU history view, physical memory used/total
  bar, and Windows commit charge/limit bar. Bars hold the last read value when
  a query fails. CPU values appear after the first refresh interval, once a
  sampling baseline exists.
- Toggle from the compact total-CPU history to a responsive grid of current
  logical-CPU meters; the header grows to accommodate the grid.
- Selection and vertical process-list scrolling.
- Flat and parent/child tree process views, with expandable process groups.
- Process sorting and live case-insensitive filtering; tree filters retain the
  ancestors of matching processes for context.
- Safe resize behavior, a 64 columns by 12 rows minimum-size message, and
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
| `c` | Toggle total CPU history and logical-CPU meters |
| `t` | Toggle flat and tree process views |
| `Enter` or `Space` | Expand or collapse the selected tree process |
| `x` | Request termination of the selected process; confirm with `y`, cancel with `n` or `Esc` |
| `s` | Cycle PID, name, user, threads, CPU, and memory sorting |
| `S` | Reverse the active sort direction |
| `/` | Edit a filter; it applies as you type |
| `Enter` | Keep the edited filter |
| `Esc` | Cancel filter editing, or quit when not editing |
| `Ctrl+U` | Clear the filter while editing |
| `q` or `Ctrl+C` | Quit |

Filtering matches process names, users, and command lines that Windows allowed
wtop to read. The table title shows the active sort and current filter.

Process CPU is normalized to total machine capacity, so it ranges from 0 to
100%, matching the CPU value in the system header.

## Terminology and access limitations

`Mem` is physical memory used/total. `Commit` is the Windows commit charge and
commit limit reported by `GetPerformanceInfo`; it is not a Linux-style swap
measurement.

Windows restricts access to some protected and system processes. Command-line
details remain available to filtering when Windows allows wtop to read them,
but are not shown in the table. Running from an elevated terminal can reveal
more details, but does not guarantee access to protected processes.

`User` is best-effort: the PID 4 System process is shown as `<kernel>` because
it has no user token. wtop labels the common service SIDs as `SYSTEM`, `LOCAL
SERVICE`, or `NETWORK SERVICE`; other unresolved identities remain as their
SID. When a process token cannot be read, wtop consults the Service Control
Manager for the service's configured account. `<restricted>` means neither
source was available, and `<unknown>` means Windows supplied no owner at all.
`Threads` is the current count from the Windows Tool Help process snapshot. If
that snapshot cannot be read, wtop keeps the prior count and marks it stale.

wtop can request termination of a selected process only after a `y` confirmation.
It refuses PID 0, PID 4, and itself; Windows access controls decide whether other
processes can be terminated. Priority changes, saved views, custom columns, and
disk/network/GPU metrics are outside this first milestone.

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
- Move through a process list taller than the terminal and verify that deeply
  nested tree names remain readable.
- Toggle tree mode with `t`, expand/collapse with `Enter` or `Space`, then sort
  with `s`/`S` and filter with `/`; accept, cancel, and clear a filter.
- Select a disposable test process, press `x`, cancel with `n`, then repeat and
  confirm with `y`; ensure protected processes report an access error.
- Resize the terminal, including to 64 columns by 12 rows, then quit with
  `q`, `Esc`, and `Ctrl+C` to confirm terminal restoration.
- Confirm protected processes remain visible even when command-line details are
  unavailable.
