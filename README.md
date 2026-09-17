# wtop

`wtop` is an htop-style system monitor for Windows terminals

## Prerequisites

Install the current stable Rust toolchain from [rustup](https://rustup.rs/).

## Run during development

```powershell
cargo run
```

Press `q` or `Esc` to exit. `Ctrl+C` also exits. The application uses the
alternate screen buffer and restores the terminal on exit.

## Install the `wtop` command locally

```powershell
cargo install --path .
```

Ensure Cargo's bin directory is on your `PATH`, then run:

```powershell
wtop
```