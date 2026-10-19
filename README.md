# Kurasi

Kurasi is a personal reference-curation tool for Windows 10 (version 1809 or later) and
Windows 11, 64-bit. It scans a project folder, shows the files as a catalogue, lets you highlight
PDFs, Markdown notes and images with an analysis per highlight, and gives an AI agent the same
catalogue through an MCP server. Version 0.4.1 is a Rust workspace with a Tauri 2 window.
`docs\CONTRACT.md` sets the behaviour and `docs\CONTRACT_RUST.md` the stack; where they differ,
`CONTRACT_RUST.md` wins. The earlier Python build is kept at git tag `python-v0.1`.

## Install

`src-tauri\build.ps1` writes two deliverables to `release\`:

| File | Use |
| --- | --- |
| `Kurasi-<version>-Setup.exe` | Wizard for the current user, with no admin rights. Default folder `%LOCALAPPDATA%\Programs\Kurasi`; any folder works, a USB drive included. Options: portable mode (on), Start Menu shortcut (on), Desktop shortcut (off), `kurasi` on the user PATH (off). Installs the WebView2 runtime when it is missing. |
| `Kurasi-<version>-portable.zip` | The same program folder with portable mode on. Unzip it and run `kurasi.exe`. |

`release\SHA256SUMS.txt` lists the SHA-256 checksum of both files. To check a copy, compare the
line with the output of `Get-FileHash <file>` in PowerShell (the same digits in capitals), or run
`sha256sum -c SHA256SUMS.txt` in Git Bash in the folder that holds the files.

Portable mode is the file `portable.txt` next to `kurasi.exe`: settings and projects then live in
`data\` beside it. Without that file they live in `%LOCALAPPDATA%\Kurasi`. The environment variable
`KURASI_HOME` overrides both.

## Use Kurasi with Claude Code (MCP)

This tutorial connects Kurasi to Claude Code, Anthropic's AI assistant for the terminal. You type
a few commands and copy text out of Kurasi. No programming is needed.

### 1. What you get

Claude Code can search your catalogue, read your PDFs and Markdown notes, and propose metadata,
categories and tags. In the mode `write` it can also add highlights with an analysis. Kurasi marks
everything Claude does, keeps it for your review, and records every call in the AI-use log.

Claude Code starts the Kurasi MCP server by itself whenever it needs it. The Kurasi window may be
open or closed. When it is open, it shows Claude's changes within about two seconds.

### 2. Before you start

1. **Install Kurasi** (see [Install](#install)). Write down the folder that holds `kurasi.exe`:
   - Setup exe: `C:\Users\<you>\AppData\Local\Programs\Kurasi` (the wizard's default,
     `%LOCALAPPDATA%\Programs\Kurasi`), or the folder you chose in the wizard.
   - Portable zip: the folder you unzipped it into.
2. **Open your project in Kurasi once.** Choose **File, New project** (Ctrl+N), click **Browse**
   to pick the folder in the Windows folder picker, and wait until the first scan finishes. The scan fills the catalogue that Claude reads. In the
   modes `read` and `propose` Claude cannot scan the folder itself.
3. **Install Claude Code.** Open PowerShell (Start menu, type `PowerShell`, press Enter) and run:

   ```
   irm https://claude.ai/install.ps1 | iex
   ```

   Open a new PowerShell window and run `claude --version`. It prints a version number when the
   install worked. Run `claude` once and follow the browser prompts to log in. Claude Code needs
   a Pro, Max, Team, Enterprise or Console account. The full instructions and the other ways to
   install are on Anthropic's [Claude Code setup page](https://code.claude.com/docs/en/setup).

### 3. Register Kurasi with Claude Code

Registration tells Claude Code how to start Kurasi for one project folder. You do it once per
project.

**The easy way: copy the command from Kurasi**

1. Open the project in Kurasi.
2. Choose **Tools, MCP server**, or click **MCP (stdio)** at the right end of the status bar.
3. Click **Copy registration command**. The line has this form:

   ```
   claude mcp add kurasi -s user -- "C:\Users\<you>\AppData\Local\Programs\Kurasi\kurasi.exe" mcp --project "D:\Research\My project" --mode write
   ```

4. Paste the line into PowerShell and press Enter.

`-s user` stores the server in your user scope, so it works in every folder where you start
Claude Code. `--mode write` lets Claude use every Kurasi tool (see **Choose the mode** below).

The copied line names the `kurasi.exe` of the window you have open. Claude therefore works on the
same catalogue you see.

**By hand: fill in the template**

```
claude mcp add kurasi -s user -- "<install dir>\kurasi.exe" mcp --project "<project folder>" --mode write
```

- `<install dir>` is the folder from step 2. For a development build it is `<repo>\target\release`.
- `<project folder>` is the folder you opened as a project in Kurasi.
- Keep the double quotes around both paths. A path with a space in it breaks without them.
- Everything after `--` is the command Claude Code runs to start Kurasi.

**Choose the mode**

`--mode` sets what Claude may do, and it is set when you register. Use `write`, as the copied
line does; it is also the mode when `--mode` is left out. `propose` and `read` are stricter
options for a project where Claude should only suggest, or only look.

| Mode | Claude can | Your PDF files |
| --- | --- | --- |
| `read` | search the catalogue and read references, pages, notes, highlights, tags and categories | unchanged |
| `propose` | everything in `read`, plus propose metadata, categories, tags for a highlight and the deletion of a highlight; each proposal waits for your review | unchanged |
| `write` | everything in `propose`, plus scan the folder, add highlights with an analysis, write exports to `_exports\` and add entries to the AI-use log | Claude's highlights are saved in them as ordinary annotations |

In every mode Kurasi marks what Claude did and records each call in the AI-use log, so you can
review it. To change the mode later, see step 8.

### 4. Check the connection

1. In PowerShell, run:

   ```
   claude mcp list
   ```

   The line for `kurasi` says **Connected**. **Failed to connect** means that Claude Code could
   not start Kurasi: see [Troubleshooting](#9-troubleshooting).
2. Run `claude mcp get kurasi` to see the details of the registration. Check the path to
   `kurasi.exe`, the project folder and the mode.
3. Start Claude Code with `claude`, type `/mcp` and press Enter. The panel lists `kurasi` with its
   status and its number of tools. Kurasi lists all 14 tools in every mode. The mode decides
   which calls Kurasi accepts.

Claude Code can ask your permission before it calls a Kurasi tool. Read the request and approve
it. Anthropic's page [Connect Claude Code to tools via MCP](https://code.claude.com/docs/en/mcp)
explains these commands, the scopes and the `/mcp` panel in full.

### 5. First tasks

Kurasi writes ready prompts for you. Click a **Copy prompt for Claude** button, paste the prompt
into Claude Code and press Enter. The prompt names the project, the reference IDs and the tools
to use. The buttons appear while **Tools, Settings, Notify when an MCP tool can do the current
step** is on, which is the default.

| Task | Where the button is | Mode needed | Where the result shows |
| --- | --- | --- | --- |
| Label new references | above the catalogue, next to "N uncategorized" | `propose` | a blue dot on each card Claude labelled |
| Fill one reference | in the edit pane: right-click a card, **Edit metadata** | `propose` | the fields of that edit pane |
| Mark key passages | in an open document: the first time you click **Highlight** in the toolbar, a notice with the button shows for a few seconds | `write` | blue highlights in the document and in the knowledge pane on the right |

**Label new references.** The prompt lists every uncategorized reference:

```
Use the Kurasi MCP server for the project "My project". For each reference below, read page 1 with reference_text, then call reference_update_metadata with the APA 7 fields (title, authors, year, date, publication) and reference_categorize with the best category path from categories_list. Each change waits for my review. Purpose: label new references.
Reference IDs: r-0123456789ab, r-0123456789ac
```

**Fill one reference.**

```
Use the Kurasi MCP server for the project "My project". Read page 1 of reference r-0123456789ab ("<title>") with reference_text, then call reference_update_metadata(ref_id="r-0123456789ab", ...) with its APA 7 fields: title, authors, year, date, publication, volume, issue, pages, DOI or URL, language. Purpose: fill the metadata for my review.
```

**Mark key passages.** The copied prompt ends with "My question:". Type your question after it
before you press Enter.

```
Use the Kurasi MCP server for the project "My project". Read reference r-0123456789ab ("<title>") page by page with reference_text and call highlight_add for each passage that bears on my question, with an analysis that starts with (F), (I) or (P) and tags from tags_list. Purpose: mark the key passages for my review. My question: <your question>
```

The **New highlight** dialog has one more button. Its prompt asks Claude to add the passage you
selected, with a drafted analysis. It also needs `write`.

You can also ask in your own words, for example: "Use the Kurasi MCP server and find the
references about <topic>."

### 6. Review what Claude proposed

Metadata and category proposals take effect at once and carry a mark, so you see them in place.
Tag and delete proposals wait for your click. You decide what stays.

- **References.** A reference Claude changed has the status Suggested and a blue dot on its card.
  Right-click the card and choose **Edit metadata**. The pane says "Proposed by Claude via MCP.
  Save to accept, or edit first." Correct any field and click **Save**. The status becomes
  Verified and the dot turns green.
- **A proposed deletion.** Open the document. In the knowledge pane the highlight carries the line
  "Claude via MCP proposes to delete this highlight", with Claude's reason. Click **Keep it** or
  **Delete highlight**.
- **Proposed tags.** They appear as dashed chips such as `+ #method` under the highlight in the
  knowledge pane. Click a chip to accept that tag.
- **Highlights Claude added** (mode `write`). They are drawn in blue, and the knowledge pane shows
  "Claude via MCP" as their author. Edit them like your own, or delete one with the cross on its
  header. Ctrl+Z undoes the last delete. Other PDF readers show them as ordinary highlights.

The name Claude comes from **Tools, Settings, AI author name for MCP writes**.

**Live refresh.** While the window is open, Kurasi checks for changes every two seconds. Each
change by Claude shows a short notice that starts with "Claude via MCP:".

**The AI-use log.** **Tools, MCP server** lists the recent calls under "Recent calls (the AI-use
log)", with the time, tool, client, purpose and result. Refused calls are listed too, with the
reason. The dialog reads the log when it opens, so reopen it to see new calls.
**Export log (Markdown)** writes the log to `.kurasi\AI_USE_LOG.md` in the project folder.

### 7. Several projects

**In Kurasi.** **File, Projects** (Ctrl+O) lists every project, the most recently opened first,
with its name, folder and when you last opened it. Type to search by name or folder, move with
Up and Down, and press Enter to open. **Open new folder** starts a project from another folder
through the Windows folder picker. **Remove** takes a project off the list; its folder and your
files stay where they are. Tick **Also delete Kurasi's data for this project** to remove
Kurasi's catalogue database and Kurasi's files in `.kurasi` too; the dialog shows each folder
first. The notice after a removal has **Undo** for 15 seconds, and removed data waits 7 days in
Kurasi's trash (see [Where data lives](#where-data-lives)). The sample project has **Reset**,
which rebuilds it from scratch.

**In Claude Code.** One registration serves one project folder. For a second project, add a
second registration with its own name:

```
claude mcp add kurasi-thesis -s user -- "<install dir>\kurasi.exe" mcp --project "D:\Thesis" --mode write
```

Claude Code then starts one Kurasi server per name. Name the project in your prompt. The copied
prompts already start with `Use the Kurasi MCP server for the project "..."`. If you prefer a
single registration, remove it and add it again with the other folder (step 8).

**The window and Claude on the same project.** Both may work on one project at the same time.
Their writes take turns. When a write waits longer than 30 seconds, Kurasi refuses it with
"Another Kurasi window or the MCP server is writing to this project. Try again in a moment." Run
the step again. The window shows Claude's changes for the project it has open. Changes to another
project show when you open that project.

### 8. Remove or change the registration

To remove the registration, run:

```
claude mcp remove kurasi -s user
```

To change the folder or the mode, remove the registration and add it again with the new values,
as in step 3. A second registration is removed by its own name, for example
`claude mcp remove kurasi-thesis -s user`. Removing a registration leaves your Kurasi project, its
catalogue and its files as they are.

### 9. Troubleshooting

| Problem | Fix |
| --- | --- |
| `claude mcp list` does not show `kurasi` | Check the folder where you ran `claude mcp add`. A line without `-s user` keeps the server for that folder only. Add it again with the line from **Tools, MCP server**, and remove the old entry in that folder with `claude mcp remove kurasi -s local`. |
| `kurasi` shows **Failed to connect** | Run `claude mcp get kurasi` and check both paths. Each must exist and sit in double quotes, because a path with a space breaks without them. Kurasi writes the reason to `logs\kurasi.log` in its data folder (see [Where data lives](#where-data-lives)), for example "is neither a known project id nor an existing folder". |
| Claude finds no references, or misses files you just added | Kurasi has not scanned them yet. Open the project in Kurasi and press F5 (**File, Scan folder now**). In the mode `write` Claude can run `catalog_scan` itself. Also check that the registration uses the `kurasi.exe` of your window: an installed copy and a portable copy each keep their own data. The line from **Tools, MCP server** always names the right one. |
| Claude reports "The Kurasi MCP server runs in propose mode, so highlight_add (a write tool) is refused." | The current mode does not allow that tool. Register again with `--mode write` (step 8), or add the highlight yourself. |
| Claude reports "This project is read-only, so its files stay unchanged." | The project is read-only. Claude cannot add PDF highlights or write exports into the folder. Metadata proposals still work. To allow the write, turn off **Tools, Settings, Read-only project**. |
| **Tools, MCP server** says "No MCP calls in this project yet." | The dialog shows the log of the project open in the window. Open the project Claude works on, and reopen the dialog after Claude's calls. If it stays empty, Claude Code runs another `kurasi.exe` with its own data (see the third row). |

### 10. Reference: tools and resources

Each mode includes the tools of the modes above it in this table. Every call, refused calls
included, goes into the AI-use log with the client name Claude Code reports.

| Tool | Mode | Does |
| --- | --- | --- |
| `catalog_search` | read | searches titles, authors, tags, publications, register IDs, categories and highlight text; best matches first |
| `reference_get` | read | metadata, files and highlight count of one reference |
| `reference_text` | read | the text of one PDF page or of a whole Markdown note, with word positions |
| `highlights_list` | read | highlights, filtered by reference, tag or author |
| `tags_list` | read | the tags in use, with their references and highlight counts |
| `categories_list` | read | the category tree with full paths such as `Management/Administration` |
| `reference_update_metadata` | propose | applies metadata, authors and categories; the status becomes Suggested until you save |
| `reference_categorize` | propose | adds categories to references and creates missing ones; the status becomes Suggested |
| `highlight_propose_delete` | propose | marks a highlight for deletion with a reason; you keep or delete it |
| `highlight_suggest_tags` | propose | proposes tags for a highlight; you accept each tag |
| `catalog_scan` | write | scans the project folder; `full=true` rehashes every file |
| `highlight_add` | write | adds a highlight with analysis, label and tags to a PDF, Markdown note or image, with the AI as author |
| `export_references` | write | writes `apa-md`, `bibtex`, `csl-json` or `highlights-json` under `_exports\` |
| `log_append` | write | adds an entry to the AI-use log with the purpose and the checks performed |

| Resource | Holds |
| --- | --- |
| `kurasi://references` | every reference with metadata, files and highlight counts |
| `kurasi://reference/{ref_id}` | one reference with its detail fields |
| `kurasi://reference/{ref_id}/highlights` | the highlights of one reference |
| `kurasi://tags` | the tags in use |

In Claude Code, type `@` to attach a resource to your message, for example `@kurasi:kurasi://tags`.

## Run

`kurasi.exe` with no arguments opens the window. A splash shows while the last opened project
loads, then the main window takes over. The local server listens on `127.0.0.1:7343`, or the next
free port, and answers only requests addressed to that host and port.

| Command | Does |
| --- | --- |
| `kurasi.exe [app] [--port N]` | desktop window |
| `kurasi.exe mcp --project <folder or id> [--mode write\|propose\|read]` | MCP server over stdio; mode `write` by default, `propose` and `read` are stricter |
| `kurasi.exe scan --project <folder or id> [--full] [--read-only] [--name NAME]` | one scan, printed as JSON; a new folder is registered as a project |
| `kurasi.exe demo` | build or rebuild the sample project |
| `kurasi.exe export --project <folder or id> --format apa-md\|bibtex\|csl-json\|highlights-json [--ref-ids ID ...]` | write an export under `<folder>\_exports\` |
| `kurasi.exe serve [--port N] [--no-startup] [--no-watch]` | the local server without a window; `--no-startup` leaves the last project closed, `--no-watch` turns the folder watcher off |

`kurasi.exe` is a GUI-subsystem program. From PowerShell, `Start-Process -Wait` returns its exit
code.

A read-only project keeps every file in its folder unchanged: PDF annotations, page rotation,
the sidecar and exports are refused, while catalogue edits still go to the database. Online-only
OneDrive files are listed from their directory entry and are never opened, hashed or
thumbnailed, so the scan does not download them.

## Where data lives

`<root>` is `KURASI_HOME`, `data\` in portable mode, or `%LOCALAPPDATA%\Kurasi`.

| Place | Holds |
| --- | --- |
| `<root>\projects.json` | the list of projects: id, name, folder, created, last opened |
| `<root>\projects\<id>\kurasi.sqlite` | the catalogue of one project (SQLite, WAL), with `write.lock` beside it |
| `<root>\thumbs\`, `<root>\pages\` | thumbnail and page render caches, keyed by file hash (pages trimmed to 500 MB) |
| `<root>\demo\` | the sample project used by the guided tour |
| `<root>\logs\kurasi.log` | the log |
| `<root>\trash\projects\rm-<id>\` | data of a project removed with **Also delete Kurasi's data**: the database folder and Kurasi's files from `.kurasi`, with `removed.json` naming where each came from. Undo moves them back. Kurasi deletes them at its first start after 7 days |
| `<folder>\.kurasi\metadata.json` | the sidecar: references, files, metadata, categories, Markdown and image highlights |
| `<folder>\.kurasi\AI_USE_LOG.md` | the AI-use log export |
| `<folder>\_exports\` | reference and highlight exports |
| the PDF files | PDF highlights, as ordinary annotations that any PDF reader shows |

## Build

Requirements: Rust stable (MSVC) through `rustup`, Visual Studio 2022 Build Tools,
`cargo install tauri-cli --version "^2"`, and `src-tauri\resources\pdfium.dll` (gitignored;
`PDFIUM_SOURCE.md` beside it says where it came from). The Tauri bundler downloads NSIS on the
first build.

```
$env:CARGO_BUILD_JOBS = 4
cargo test --workspace
cargo build --release
powershell -File src-tauri\build.ps1
```

Build one target at a time with `CARGO_BUILD_JOBS=4`: parallel release builds run this PC out of
memory. `build.ps1` runs `cargo tauri build`, stages the portable folder under
`target\release\bundle\portable\`, and writes the Setup exe, the portable zip and `SHA256SUMS.txt`
to `release\`.
`-SkipCargo` reuses the last `cargo tauri build` output. Debug runs: `cargo run -p kurasi -- app`
or `cargo run -p kurasi -- mcp --project <folder>`.

Set `KURASI_HOME` to a temporary folder for every test or manual run, so the real
`%LOCALAPPDATA%\Kurasi` stays untouched.

## Layout

```
Cargo.toml              workspace, version 0.4.1
CHANGELOG.md            what changed in each version
LICENSE                 MIT License
crates\kurasi-core\     config and app paths, SQLite schema, scanner, search, sidecar, media,
                        sample project, export, change feed, PDF backend trait
crates\kurasi-pdf\      pdfium-render over pdfium.dll, lopdf patches for annotations
crates\kurasi-server\   axum HTTP API and the rmcp stdio MCP server
src-tauri\              kurasi.exe: CLI, native splash, Tauri window, NSIS installer, build.ps1
ui\                     the web interface; ui\dev\ holds the development scripts
docs\                   CONTRACT.md, CONTRACT_RUST.md, CORE_API.md
release\                build.ps1 output (gitignored)
```

## Development

- UI screenshots: `node ui\dev\shoot.mjs [scenario ...]` drives headless Chrome against the fake
  API in `ui\dev\fake_api.js` and writes to `ui\dev\shots\`. Node 24 is installed under
  `D:\nodejs`.
- Integration run: `node ui\dev\integrate.mjs` drives the real interface and the MCP server. Its
  helpers run in `.venv`: `ui\dev\mcp_client.py` (MCP stdio client) and `ui\dev\pdfcheck.py`
  (prints the annotations and page rotations of a PDF as JSON).
- `cargo run -p kurasi-server --bin kurasi-mcp-test -- <project id or folder | demo> [read|propose|write]`
  runs the MCP server for a stdio client, with `KURASI_HOME` set to a temporary folder.
- `.venv` (Python 3.14) holds `pymupdf` for `pdfcheck.py` and for the PyMuPDF cross-checks in the
  Rust tests (skipped when `.venv` is absent), and `mcp` for `mcp_client.py`. Tested with pymupdf
  1.28.2 and mcp 2.3.0:

```
py -3.14 -m venv .venv
.venv\Scripts\python.exe -m pip install pymupdf mcp
```

## License

Kurasi is released under the MIT License (`LICENSE`). It is provided as is, without warranty of
any kind, and the author is not liable for any damage or data loss that comes from its use.
Kurasi is a personal project: it comes with no support and no promise of maintenance.

The release carries the licence in three places: `LICENSE.txt` in the install folder and the
portable zip, a licence page in the Setup wizard, and a line on the splash screen.
`THIRD_PARTY_LICENSES.txt` lists the Rust crates built into `kurasi.exe` with their licence
texts; `node src-tauri\third_party.mjs` writes it from `Cargo.lock`, and `build.ps1` runs it
before each build. The pdfium licences ship in `licenses\pdfium\`.
