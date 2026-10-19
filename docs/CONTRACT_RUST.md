# Kurasi: Rust and Tauri 2 rewrite (addendum to CONTRACT.md)

Decided by the user on 17 Oct 2026. `CONTRACT.md` stays binding for behaviour (storage,
schema, scan rules, PDF writes, HTTP API, MCP tools, UI rules, sterile demo, testing rules,
writing rules, splash and last project). This file changes the stack and adds three
features. Where the two files differ, this file wins.

## 1. Why

The Python build starts slowly (interpreter plus PyMuPDF plus pywebview: 2 to 4 s before
the first window, 19 s cold import of `mcp`) and the splash hand-over to the main window
failed with WebView2 `E_ABORT`. The user wants a compact language. Choice: Rust with
Tauri 2, HTML/JS interface kept.

## 2. Stack

| Part | Choice | Notes |
| --- | --- | --- |
| Shell | Tauri 2 (`tauri`, `tauri-build`), WebView2 | Two windows in `tauri.conf.json`: `splash` and `main` |
| Web server | `axum` 0.8 on `tokio`, bound to 127.0.0.1, port 7343 or next free | Same routes as CONTRACT.md section 6. The main window loads `http://127.0.0.1:<port>/ui/`. Host and Origin check kept |
| Compression | `tower-http` `CompressionLayer`, feature `compression-gzip`, fastest level | gzip for JSON, `text/*`, JavaScript and SVG bodies of 1 KB or more when the request accepts it; page images, thumbnails and file bytes pass as they are (CONTRACT.md section 6). WebView2 and Chrome send `Accept-Encoding: gzip` |
| Database | `rusqlite` with `bundled` feature, WAL | Schema of CONTRACT.md section 3 unchanged, so a Python-made `kurasi.sqlite` opens as is |
| PDF | `pdfium-render` with `pdfium.dll` from bblanchon/pdfium-binaries (x64, non-V8) shipped in `src-tauri/resources/` | Render pages and thumbnails, words with boxes, read and write highlight annotations (author in `/T`, contents, colour, quad points), rotate, save to temp then `std::fs::rename` with the six-retry rule. If `pdfium-render` cannot write an annotation field, patch that field with `lopdf`; no third PDF library. The page routes reuse an open document (`DocCache`, CORE_API.md) and run at most two at a time (CONTRACT.md section 6) |
| Images | `image` crate | Thumbnails, rotation, region highlights stored in the sidecar |
| File watch | `notify` with `notify-debouncer-full`, 2 s debounce | |
| Hashing | `sha2` | Streaming SHA-256 |
| Search | Own module, port of `search.py` (bigram Dice, loose and strict) | Also `FullText`: the index behind `/api/search`, with the UI's token, word and bigram rules (CONTRACT.md section 8), built on first use and kept in step with the change feed |
| MCP | `rmcp` (official Rust SDK), stdio transport | Tool list of CONTRACT.md section 7 unchanged |
| CLI | `clap` | `kurasi.exe` with no arguments starts the app; `kurasi.exe mcp --project <dir> [--mode write\|propose\|read]` (write by default), `scan`, `demo`, `export`, `serve` as in the Python CLI |
| Dialogs, opener | `tauri-plugin-dialog`, `tauri-plugin-opener` | Folder picker: the Windows select-folder dialog, which the page opens with `window.__TAURI__.dialog.open({directory:true})` (`withGlobalTauri`) for Browse and Open new folder. `capabilities\main.json` grants the plugin `dialog:allow-open` only, for the `main` window and the local server URLs (`http://127.0.0.1:*`, `http://localhost:*`); message, ask, confirm and save stay closed. Opener: open with the Windows default app |

The `mcp`, `scan`, `demo`, `export` and `serve` subcommands never initialise Tauri or a
window. MCP cold start target: under 500 ms to the first `initialize` response.

## 3. Layout

```
D:\PythonScripts\Kurasi\
  Cargo.toml                 workspace
  crates\kurasi-core\        library: config, db, scanner, media, search, sidecar, lock,
                             demo, export, change feed, PDF backend trait
  crates\kurasi-pdf\         pdfium-render over pdfium.dll, lopdf patches for annotations
  crates\kurasi-server\      axum API and the rmcp stdio MCP server
  src-tauri\                 binary `kurasi`: cli.rs (clap), app.rs (windows), splash.rs,
                             tauri.conf.json, capabilities\, icons\, resources\pdfium.dll,
                             installer\ (NSIS template), build.ps1
  ui\                        the web interface, unchanged location
  docs\                      CONTRACT.md, CONTRACT_RUST.md, CORE_API.md
  release\                   build.ps1 output (gitignored), see section 6
```

The Python reference build (`kurasi\`, `tests\`, `Kurasi.cmd`, `requirements.txt`,
`pytest.ini`) was removed from the working tree on the user's request on 18 Oct 2026 and stays
recoverable from git tag `python-v0.1`.

Rust tests live in each crate (`cargo test`). The UI tests stay as they are
(`ui\dev\shoot.mjs` against `fake_api.js`).

## 4. Startup and splash (replaces CONTRACT.md section 12 mechanics, keeps its rules)

1. `kurasi.exe` starts, parses arguments, and if the app is wanted, builds the Tauri app.
2. `tauri.conf.json` declares `splash` (decorations false, 960x600, centered, visible,
   `/ui/splash.html` served from the bundled assets, so it needs no server) and `main`
   (visible false, 1600x900, min 960x600, `label: "main"`). Window size rule, applied
   while `main` is still hidden: the startup task reads the work area (the screen minus
   the taskbar) and the scale factor of the monitor the window is on, or of the primary
   monitor. When the 1600x900 window with its frame fits within 92 percent of the work
   area in both dimensions, it opens at that size. Otherwise each dimension that overflows
   shrinks to 92 percent of the work area and the window opens maximized, so restoring it
   gives the fitted size. The window is centred on the work area, and its minimum size is
   clamped to the fitted size so it never exceeds the screen. The native splash
   (`splash.rs`, 960x600 times the system DPI) shrinks with its aspect kept to the same
   92 percent of the primary monitor's work area and is centred on that work area.
3. In `setup`, a Tokio task opens the last project (`projects.json` `last_opened`), starts
   axum, runs the incremental scan, and builds the tag graph layout. It reports progress
   with `app.emit("splash-status", text)`; `splash.html` listens through
   `window.__TAURI__.event.listen`.
4. When ready, and at least 2000 ms after the splash was shown, the task sets the main
   window URL to the server, waits for the main window's `tauri://created` plus the page's
   `kurasi-ready` event (sent by `app.js` after the first render), then calls
   `main.show()`, `main.set_focus()`, and `splash.close()` in that order. Safety: after
   30 s the main window is shown regardless and the splash closes.
5. Any error during startup is shown on the splash with a Retry and a Quit button.

Target: splash visible within 300 ms of the double-click; main window within 2.5 s for a
project of 300 references on a warm cache.

## 5. Tag graph controls (new)

The Tag graph tab gets the standard canvas controls. All state persists in the `setting`
table of the project (`graph.zoom`, `graph.pan_x`, `graph.pan_y`, `graph.font_px`,
`graph.label_opacity`, `graph.edge_opacity`, `graph.spacing`).

The graph shows every tag (circle), author (diamond) and paper (square) in the current filter.
The canvas carries the toolbar and the drawing only: the type checkboxes, the "at least N
papers" slider and the hint line above the canvas were removed on 18 Oct 2026.

- Zoom: mouse wheel zooms around the cursor, 0.2x to 6x. Toolbar buttons `+`, `-`, `Fit`
  (fits all nodes and their labels with 24 px padding, below the toolbar), `1:1`. Keyboard:
  `Ctrl +`, `Ctrl -`, `Ctrl 0` (fit). When no view is saved yet, or when no node is inside
  the canvas, the first drawing fits the graph without zooming in past 100%.
- Node spacing: slider 50 to 300 percent, default 100, in steps of 10. It is an input of the
  layout: the value applies when the slider is let go, the graph is laid out again around the
  centre of the canvas and the zoom stays, so the marks keep their size on screen and the
  gaps between them change.
- Mark size: papers have a radius of 6 px. Tags and authors grow with the number of papers
  they touch on a scale defined up to 10,000 papers:
  r = 6 + 22 x (min(4, log10(papers)) / 4)^0.75, rounded to 0.1 px. Each tenfold step adds
  less than the one before and counts inside one decade still differ: 1 paper 6, 10 papers
  13.8, 100 papers 19.1, 1000 papers 23.7, 3000 papers 25.8, 7000 papers 27.4, 10,000 papers
  and more 28.
- Paper labels: the first three words of the title and 24 letters at most, cut at a word end
  when one is near, with an ellipsis when cut. The hover tooltip shows the full title.
- Layout: no two marks overlap. Every force scales with the ideal distance, which fills the
  canvas for a small graph and never drops below the room two average marks need; a link to
  a busy tag rests longer, so its papers fit around it. Repulsion runs over a Barnes-Hut
  quadtree (buckets of 8 bodies, opening angle 0.9, and 1.1 above 1000 nodes) and collisions
  over a spatial hash, so one round costs about n log n. A collision step in the second half of
  the rounds and separation passes at the end keep marks apart, and a further pass keeps the
  box of a mark and its label clear of the others in graphs of up to 300 nodes. Up to 400
  nodes the layout starts from a ring and runs 260 rounds; up to 1000 nodes the rounds fall in
  proportion to the node count. Above 1000 nodes it starts from a seeded placement (tags on a
  spiral in order of the papers they touch, each paper near the mean of its tags, each author
  near the mean of its papers) and runs from 104 rounds at 1000 nodes down to 60 at 15,000.
- Pan: left-drag on empty canvas, middle-drag anywhere, arrow keys 40 px per press.
  Dragging a node still moves the node; the node and its neighbours stay lit while it moves.
- Font size: slider 8 to 24 px for node labels, default 12. Labels scale with zoom and never
  shrink below 9 px on screen (or the chosen size when it is smaller).
- Label level of detail: once the view has rested for 120 ms, labels are placed for the nodes
  in sight in priority order (tags and authors by the number of papers they touch, then
  papers), and a label whose box would overlap one already placed is skipped. A paper label
  also needs a spot clear of other marks. At most 900 labels show. Hovering a node shows its
  label, adding it when it was skipped, and fades every other label except those of its
  neighbours. Above 1000 nodes the labels hide while the zoom changes.
- Opacity: slider 0 to 100 percent for labels, default 100; slider 10 to 100 percent for
  edges, default 60.
- The toolbar sits at the top of the graph canvas, collapses into a single gear button
  below 1100 px width. A `Reset view` entry restores defaults.
- The force layout runs in `graph_worker.js`, one job at a time; a newer request for the same
  filter replaces one still waiting. Zoom and pan never change node coordinates. The worker
  lays the graph out around the centre of the canvas it was asked for; a canvas of another size
  only shifts the drawing.
- Drawing: edges and marks are painted on two canvases larger than the view by 30 percent on
  every side, marks as sprites drawn once per type and size on screen. While the view moves,
  both canvases follow with a CSS transform; they are painted again when the view runs past
  them and once the view rests at another zoom. Edge opacity and the hover fade are CSS
  opacity on the canvases. The SVG above them holds one invisible group per node, which takes
  the pointer and keeps the selectors the tests and the tour use (`.gbox .node.tag .mark`,
  the label in a `desc` element), plus the hover copies, the hovered node's edges and the
  labels. While the view moves the svg element follows with a CSS transform, and the view is
  written into its `.view` group once the view rests. Opening the Tag tab again with the same
  layout and canvas size puts the existing drawing back.
- Scale: the graph is built for about 15,000 nodes (the papers, authors and tags of a
  10,000-file library; see CONTRACT.md section 12). It is laid out in a few seconds, and pan
  and zoom keep the display frame rate. Measured on 18 Oct 2026 on the development laptop under
  load with generated libraries (`KURASI_BIG_REFS=<n> node ui\dev\shoot.mjs biggraph`): 444
  nodes lay out in 0.2 to 0.9 s, 5,900 nodes in 1.6 to 3.1 s and 15,100 nodes in 3 to 12 s, as
  the load on the laptop changed; pan and zoom ran at 17 to 30 ms a frame at 5,900 and 15,100
  nodes, against 300 ms or more when the marks were SVG shapes.

## 6. Portable installation wizard (new)

Two deliverables, written by `src-tauri\build.ps1` to `<repo>\release\` (gitignored, created
when missing). The script runs `cargo tauri build`, copies the bundler's
`target\release\bundle\nsis\Kurasi_<version>_x64-setup.exe` under the first name below, and
stages the portable folder in `target\release\bundle\portable\Kurasi\` before zipping it.

1. `Kurasi-<version>-Setup.exe`: NSIS wizard produced by the Tauri bundler with a custom
   template at `src-tauri\installer\installer.nsi` based on the Tauri default. Pages in
   order: welcome; licence (the MIT text of `LICENSE`, from `bundle.licenseFile`); requirement
   check; install folder; options; progress; finish.
   - Requirement check: Windows 10 version 1809 (build 17763) or later, 64-bit. Earlier
     versions stop with the message "Kurasi needs Windows 10 version 1809 or Windows 11."
     WebView2 runtime: detected through the registry keys Tauri checks; if absent, the
     bundled evergreen bootstrapper runs (`webviewInstallMode: embedBootstrapper`).
   - Install folder: default `%LOCALAPPDATA%\Programs\Kurasi` for the current user, any
     folder allowed, including a USB drive. No admin rights, no HKLM writes.
   - Options: "Portable mode: keep settings and projects inside the install folder"
     (default on), "Start Menu shortcut" (default on), "Desktop shortcut" (default off),
     "Register the `kurasi` command on PATH for this user" (default off).
   - Portable mode writes the file `portable.txt` next to `kurasi.exe`. At start, the app
     uses `<exe folder>\data\` as its app-data root when that file exists, else
     `%LOCALAPPDATA%\Kurasi` (CONTRACT.md section 2). Both roots hold the same layout.
   - Uninstaller: removes program files and shortcuts, asks before removing `data\`.
2. `Kurasi-<version>-portable.zip`: the same folder with `portable.txt` already present,
   for users who want no wizard. Unzip and run.

`SHA256SUMS.txt` beside them holds one line per file (lowercase SHA-256, two spaces, file name),
the format `sha256sum -c SHA256SUMS.txt` checks.

Both contain `kurasi.exe`, `pdfium.dll`, `ui\` assets (bundled into the exe by Tauri; the
folder copy is for the `serve` fallback) and `README.txt` with the two commands the user
needs (start, register the MCP server in Claude Code) and a licence note. Both also carry
`LICENSE.txt` (MIT) and `THIRD_PARTY_LICENSES.txt`, the licence texts of the Rust crates linked
into `kurasi.exe`, which `src-tauri\third_party.mjs` writes from `Cargo.lock` before each build.
Its last section, "Components outside Cargo.lock", holds the notices kept in
`src-tauri\installer\third_party_static.txt` (Rust standard library, WebView2 SDK loader, Tauri
and NSIS parts of the installer).
The native splash names the version and the licence at the right end of its status band.

Version comes from `Cargo.toml`; the first Rust release is `0.3.0`.

## 7. Entry point icon

Source `ui\assets\icon.svg` (1024x1024). `cargo tauri icon ui\assets\icon.svg` generates
`src-tauri\icons\` (ico, png set). The ico also serves the installer and the shortcut.
`ui\assets\icon.svg` is Kurasi's own work, covered by Kurasi's MIT licence (`LICENSE`).

## 8. Build and tooling on this PC

- Rust stable MSVC via `rustup` (user-level install at `%USERPROFILE%\.cargo`), Visual
  Studio 2022 Build Tools present at `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools`,
  WebView2 runtime 155 present, Node 24 present (needed only for `ui\dev\shoot.mjs`).
- `cargo install tauri-cli --version "^2"` gives `cargo tauri`.
- NSIS is downloaded by the Tauri bundler on first build; no separate install.
- Debug runs: `cargo tauri dev` is unnecessary because the UI is plain files; use
  `cargo run -- app` and `cargo run -- mcp --project <dir>`.
- Agents never run `claude mcp add`, never change Claude configuration, never write under
  `D:\OneDrive\`, and never touch the user's real `%LOCALAPPDATA%\Kurasi` (set
  `KURASI_HOME` to a temp folder in every test, as the Python build did).

## 9. Parity checklist before the Python code is removed

1. Opens a `kurasi.sqlite` made by the Python build and shows the same reference count.
2. Read-only scan of `D:\OneDrive\Study\Essay\10_reading` gives the
   same file, reference and highlight counts as the Python run, with before and after
   `stat` proof that nothing under `D:\OneDrive\` changed.
3. Highlight add, edit, delete and page rotate round-trip through pdfium and are read
   back by PyMuPDF with the same author, colour, contents and quads (one Python check run
   from the legacy venv is allowed for this).
4. MCP: `initialize`, `tools/list`, `catalog_search`, `highlight_add` in write mode (and
   refused in propose mode),
   `log_append` work from a stdio client; cold start under 500 ms.
5. Splash shows within 300 ms, main window takes over without a flash, no WebView2 error
   in the log.
6. Tag graph: zoom, pan, font and opacity controls work and persist.
7. Installer: fresh install into a temp folder in portable mode, launch, uninstall; the
   wizard refuses a simulated Windows 8 (`/D` test flag or a unit test of the NSIS
   version macro is acceptable evidence).
8. `cargo test` and the UI screenshot run pass.

## 10. Writing rules

Section 11 of `CONTRACT.md` applies to every file, commit message, comment and reply: no
em-dash or en-dash, no contrastive negation, an item with no data is left out.
