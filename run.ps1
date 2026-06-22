#!/usr/bin/env pwsh
# Local dev launcher for termherd, tracked in the repo.
# Runs the GUI binary from the repo root. Extra args are forwarded to cargo run.
#
#   ./run.ps1                 # debug build
#   ./run.ps1 --release       # release build
#   ./run.ps1 -- --some-flag  # pass flags through to the app

$ErrorActionPreference = 'Stop'
Set-Location -Path $PSScriptRoot
cargo run -p termherd-app @args
