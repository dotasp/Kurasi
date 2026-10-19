# Kurasi: notes for any AI working in this repo

Kurasi is a personal reference-curation tool for Windows 10 and 11: it scans a project folder,
shows the files as a catalogue, highlights PDFs, Markdown and images, and exposes the catalogue
to an AI agent through an MCP server. Since 18 Oct 2026 the product is the Rust workspace with
Tauri 2 (version 0.4.1). The Python build is kept for reference only, as the local git tag `python-v0.1`;
its files are gone from the working tree. `README.md` describes the Rust build.

## Read first

- `docs\CONTRACT.md`: behaviour, HTTP routes (section 6), MCP tools (section 7), sample project (section 9)
- `docs\CONTRACT_RUST.md`: Rust stack, splash flow, tag graph controls, portable installer, parity checklist
- `docs\CORE_API.md`: crate interfaces; change this file before changing the code

## Layout

- `crates\kurasi-core`: config and app paths, SQLite schema (same as the Python `db.py`), scanner,
  search, sidecar, media, demo project, export, change feed, PDF backend trait
- `crates\kurasi-pdf`: pdfium-render over a dynamic `pdfium.dll` plus lopdf patches for annotations
- `crates\kurasi-server`: axum API and the rmcp stdio MCP server (14 tools, 4 resources; modes read, propose, write; default write)
- `src-tauri`: CLI (`app|mcp|scan|demo|export|serve`), native Win32 splash, Tauri window, NSIS installer, `build.ps1`
- `ui\`: the web interface served by the server; `ui\dev\shoot.mjs` takes the screenshot suite (Node at `D:\nodejs`)
- `src-tauri\resources\pdfium.dll` is gitignored; `PDFIUM_SOURCE.md` beside it says where it came from

## Hard rules

- Never write under `D:\OneDrive\`. Tests, agents and manual checks use copies elsewhere.
- Never touch the user's real `%LOCALAPPDATA%\Kurasi`. Set `KURASI_HOME` to a temporary folder for every run.
- Sample and test data stay sterile: no real names, titles, quotes or project text.
- Never run `claude mcp add` or `claude mcp remove`, and never change Claude configuration. The user does that.
- Build one target at a time with `CARGO_BUILD_JOBS=4`; parallel release builds have been killed for memory on this PC.
- `kurasi.exe` is a GUI-subsystem binary. From PowerShell use `Start-Process -Wait` to get exit codes. Its console subcommands attach to the parent console (`src-tauri\src\console.rs`): in a terminal `scan` prints the scan JSON and an INFO line, and redirected stdout receives the JSON.
- Trend Micro blocks launching the Setup exe from an agent shell. Installer runs are the user's.
- pdfium is wrapped in a process-wide lock; do not call it from several threads at once.
- Commit messages end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- The global writing rules apply to every file: no em or en dash, no contrastive negation.

## Commands

```
cargo test --workspace
cargo build --release
powershell -File src-tauri\build.ps1          # Setup exe and portable zip in release\ (gitignored)
target\release\kurasi.exe mcp --project <folder> --mode read|propose|write
target\release\kurasi.exe scan --project <folder> --read-only
```

The user registers the MCP server with
`claude mcp add kurasi -s user -- "<repo>\target\release\kurasi.exe" mcp --project "<folder>" --mode write`.

`ui\dev\mcp_client.py` starts the server with `--mode write`, the default of `kurasi.exe mcp`; the integration run calls `highlight_add`, a write tool.

## State on 19 Oct 2026

- Tag `v0.4.1` (GitHub Release with the Setup exe and the portable zip) fixes long PDFs: Kurasi keeps the three most recently used PDFs open, renders at most two pages at a time, and the viewer cancels requests for pages that leave the view. On the fix, 125 workspace tests passed and the screenshot run wrote 67 shots; those runs came before the move to the public history, and the 0.4.1 commit itself was built without a test run at the user's request. The last integration run (113 of 113) came before the search chips and the Projects dialog; integration runs need the user's approval each time.
- Public repo https://github.com/dotasp/Kurasi starts at the orphan commit `4fec814` "Kurasi 0.4.0" (tag `v0.4.0`), a scrubbed copy of the private 0.4.0; `main` tracks `origin/main`. The old history stays on this PC: branch `private-history` at `529331c`, tags `private-v0.4.0` and `python-v0.1`. The user chose to keep them off GitHub.
- The demo sterility word list is `crates\kurasi-core\tests\forbidden_words.local.txt`, gitignored and kept on this PC only. `demo_tests` skips the word check when the file is absent.
- Open: installer wizard test by the user (parity item 7); the user re-registers the MCP server with `--mode write`; the user replaces the portable copy in `Documents` with 0.4.1. The Python code was removed on the user's request on 18 Oct 2026; `.venv` stays for PyMuPDF cross-checks, `ui\dev\pdfcheck.py` and `ui\dev\mcp_client.py`.
- Known defects: annotations without `/NM` get object-number ids that a later save may renumber; the graph toolbar overlays the top-left of the canvas.
