# pdfium.dll source

`pdfium.dll` in this folder is gitignored. This note records where it came from so the
file can be fetched again and checked.

| Item | Value |
| --- | --- |
| Project | bblanchon/pdfium-binaries (prebuilt PDFium) |
| Release tag | `chromium/8100` |
| Release published | 2026-10-12T20:25:13Z |
| Asset | `pdfium-win-x64.tgz` |
| Asset URL | https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/8100/pdfium-win-x64.tgz |
| Asset size | 3,875,218 bytes |
| Asset SHA-256 | `51621BCC1D9BE9EA448F4B84B0516862B928E3968D3FCA899AF1A6D6E2848D6D` |
| PDFium version (archive `VERSION` file) | MAJOR 157, MINOR 0, BUILD 8100, PATCH 0 |
| File taken | `bin\pdfium.dll` from the archive |
| `pdfium.dll` size | 7,512,064 bytes |
| `pdfium.dll` SHA-256 | `AE913F407E8D3F5CDBF08A9869564D647B13E132E01EB376C0A5A33B7A227A45` |
| Downloaded | 2026-10-17 |

## Licence

The archive ships `LICENSE` (MIT, Copyright 2014-2025 Benoit Blanchon) and a `licenses\`
folder with the licences of PDFium (BSD-3-Clause) and its bundled libraries (abseil, agg23,
dragonbox, fast_float, freetype, harfbuzz, icu, lcms, libjpeg_turbo, libopenjpeg, libpng,
llvm-libc, simdutf, zlib). Both are committed in `pdfium-licenses\` next to this file
(`LICENSE` as `LICENSE.txt`), copied from the archive on 2026-10-17 after its SHA-256 was
checked against the table above. The installer and the portable zip ship the folder as
`licenses\pdfium\` (`tauri.conf.json` resources and `build.ps1`).
`README.txt` in that folder is Kurasi's own note: the reference in `LICENSE.txt` to "the licenses/
directory" means `licenses\pdfium\`. Keep it when the folder is refreshed from a new archive.

## Refresh

1. Download the asset URL above (or the same asset from a newer release) into an empty
   folder and extract it with `tar -xzf`.
2. Copy `bin\pdfium.dll` to `src-tauri\resources\pdfium.dll`.
3. Run `Get-FileHash src-tauri\resources\pdfium.dll -Algorithm SHA256` and update this table.
