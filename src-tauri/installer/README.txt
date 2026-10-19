Kurasi 0.4.1

Start Kurasi
  Double-click kurasi.exe, or run from a terminal:
    "<install dir>\kurasi.exe"

Register the MCP server in Claude Code (one line, user scope)
  claude mcp add kurasi -s user -- "<install dir>\kurasi.exe" mcp --project "<folder>" --mode write
  Replace <install dir> with the folder that holds kurasi.exe and <folder> with the
  project folder Kurasi should open. --mode write lets the agent use every tool and is
  also the default. The stricter modes are propose (suggestions only) and read.
  Tools, MCP server in the Kurasi window shows this line filled in, ready to copy.

Portable mode
  The file portable.txt next to kurasi.exe keeps settings and projects in the folder
  data\ beside it. Delete portable.txt to use %LOCALAPPDATA%\Kurasi instead.
  The environment variable KURASI_HOME overrides both.

Other commands
  kurasi.exe scan --project "<folder>"       scan once and print the result
  kurasi.exe serve [--port 7343]             run the local server without a window
  kurasi.exe demo                            build the sample project
  kurasi.exe export --project "<folder>" --format apa-md
  kurasi.exe --help

Licence
  Kurasi is free software under the MIT License (LICENSE.txt in this folder). It is
  provided as is, without warranty of any kind. The author is not liable for any damage
  or data loss that comes from using it. Kurasi is a personal project: it comes with no
  support and no promise of updates. Keep backups of the folders you open with it.
  The libraries built into kurasi.exe and their licences are listed in
  THIRD_PARTY_LICENSES.txt. The pdfium licences are in licenses\pdfium.
