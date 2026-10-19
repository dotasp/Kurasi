# Changelog

All notable changes to Kurasi are recorded in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the version numbers follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] - 2026-10-18

### Added

- Search chips. Press Enter in the search box, or click the + button beside it, to keep the
  text as a filter chip. A reference must match every chip. Each chip has its own remove
  control, and Clear all filters removes the chips too.
- A finder at the top of the filter pane narrows the Author and Tag entries by name. Ticked
  entries stay in sight, and the catalogue keeps its filters.
- The Projects dialog (File, Projects..., Ctrl+O) replaces Open project. It has a search box
  over project names and folder paths, a list sorted by last use, and keyboard control.
- Remove takes a project off the list. A checkbox, unticked by default, also deletes Kurasi's
  own data for that project and first lists the exact folders it would delete. Your own files
  stay untouched. An Undo notice stays for 15 seconds and brings the project back.
- Reset rebuilds the sample project from scratch.
- Browse in the New project dialog opens the Windows folder picker. Kurasi checks every folder,
  picked or typed, explains why a folder cannot be used, and fills in the project name.
- A node spacing slider in the tag graph, from 50 to 300 percent.
- Kurasi is released under the MIT licence. The Setup wizard shows the licence as a page, the
  splash screen names the version and the licence, and THIRD_PARTY_LICENSES.txt with the
  licences of the bundled components ships in the installer and in the portable zip.
- SHA256SUMS.txt beside the Setup exe and the portable zip, to check a downloaded copy.

### Changed

- The window opens at a size that fits the screen area above the taskbar. When the default size
  does not fit, the window opens maximized, and restoring it gives a size that fits. The
  smallest window is 960 by 600. The splash screen shrinks to fit in the same way.
- The Reference and Authors views scroll in one long list without pages. More cards appear as
  you near the end, and a live refresh keeps the cards and your scroll position in place.
- Reference, Author and Tag sit in a segmented switch at the start of the tab strip. Open
  documents follow as file tabs with a type badge.
- In the tag graph, tags and authors grow with the number of papers they touch, on a scale up
  to 10,000 papers. Paper labels show at most the first three words of the title, and the
  tooltip shows the full title. Nodes no longer overlap.
- Large tag graphs are drawn on a canvas, so pan and zoom stay smooth with about 15,000 nodes.
- Large catalogues load faster. The reference list carries less data and is compressed on its
  way to the window, highlight text is searched by Kurasi in the background, and a change
  refreshes only the references it touches.
- The filter pane lists categories, authors and tags from A to Z.
- The MCP server runs in write mode by default, and the registration line in Tools, MCP server
  uses write mode. Propose and read remain available as stricter modes.
- The AI-use log names the AI client under the newer MCP protocol as well.
- The build writes the Setup exe and the portable zip to the release folder.
- 10,000 files per project is the recommended maximum. Kurasi does not enforce it. Above it,
  Kurasi keeps working and gets slower.

### Removed

- The hint text above the tag graph, the slider for a minimum number of papers, and the
  checkboxes for node shapes.
- The Python build and its launcher Kurasi.cmd. Start Kurasi with kurasi.exe or its Start Menu
  shortcut. The Python build stays available as the git tag python-v0.1.

## [0.3.0] - 2026-10-18

### Added

- The first Rust and Tauri 2 build of Kurasi. It matches the Python build (0.1) feature for
  feature: catalogue, highlights on PDFs, Markdown notes and images, tag graph, guided tour,
  sample project, and the MCP server with 14 tools.
- A native splash screen, a Setup wizard that installs for the current user without admin
  rights, and a portable zip.
