<#
.SYNOPSIS
  Builds the Kurasi release: the NSIS installer through `cargo tauri build` and the portable zip.

.DESCRIPTION
  Run from anywhere: `powershell -ExecutionPolicy Bypass -File src-tauri\build.ps1`.
  Deliverables in <repo>\release\ (gitignored, created when missing):
    Kurasi-<version>-Setup.exe      copy of the bundler's target\release\bundle\nsis\Kurasi_<version>_x64-setup.exe
    Kurasi-<version>-portable.zip   kurasi.exe, pdfium.dll, ui\, licenses\pdfium\, portable.txt, README.txt,
                                    LICENSE.txt, THIRD_PARTY_LICENSES.txt
    SHA256SUMS.txt                  SHA-256 of the two files above, one line each in the format
                                    `sha256sum -c` reads (lowercase hex, two spaces, file name)
  Both carry LICENSE (MIT; the Setup wizard shows it as a page) and THIRD_PARTY_LICENSES.txt, which
  third_party.mjs writes from Cargo.lock and installer\third_party_static.txt. The script runs it with
  Node when Node is found, and stops when Node is missing and the file is older than one of its inputs.
  The portable folder is staged in target\release\bundle\portable\Kurasi\ before it is zipped.
  Switches: -SkipCargo reuses the last `cargo tauri build` output, copies its installer and packs the zip.
#>
param(
    [switch]$SkipCargo
)

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Split-Path -Parent $here
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'User') + ';' + [Environment]::GetEnvironmentVariable('Path', 'Machine')

# Version from src-tauri's effective manifest (workspace inheritance resolved by cargo).
$meta = cargo metadata --format-version 1 --no-deps --manifest-path (Join-Path $here 'Cargo.toml') | ConvertFrom-Json
$pkg = $meta.packages | Where-Object { $_.name -eq 'kurasi' } | Select-Object -First 1
if (-not $pkg) { throw 'package kurasi was not found in the workspace metadata' }
$version = $pkg.version
Write-Host "Kurasi $version"

$pdfium = Join-Path $here 'resources\pdfium.dll'
if (-not (Test-Path $pdfium)) { throw "pdfium.dll is missing at $pdfium (see resources\PDFIUM_SOURCE.md)" }

# Third-party licences of the linked crates, refreshed before the bundler copies them.
$thirdParty = Join-Path $here 'installer\THIRD_PARTY_LICENSES.txt'
$node = Get-Command node -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -First 1
if (-not $node) { $node = Get-ChildItem 'D:\nodejs\node-v*\node.exe' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty FullName -Last 1 }
if ($node) {
    & $node (Join-Path $here 'third_party.mjs')
    if ($LASTEXITCODE -ne 0) { throw "third_party.mjs failed with exit code $LASTEXITCODE" }
} else {
    $inputs = @((Join-Path $repo 'Cargo.lock'), (Join-Path $here 'third_party.mjs'), (Join-Path $here 'installer\third_party_static.txt'))
    $newest = ($inputs | ForEach-Object { (Get-Item $_).LastWriteTime } | Measure-Object -Maximum).Maximum
    if (-not (Test-Path $thirdParty) -or (Get-Item $thirdParty).LastWriteTime -lt $newest) {
        throw "THIRD_PARTY_LICENSES.txt is older than Cargo.lock, third_party.mjs or third_party_static.txt and Node was not found to run src-tauri\third_party.mjs"
    }
}

if (-not $SkipCargo) {
    Push-Location $here
    try {
        cargo tauri build
        if ($LASTEXITCODE -ne 0) { throw "cargo tauri build failed with exit code $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
}

$release = Join-Path $repo 'target\release'
$bundle = Join-Path $release 'bundle'
$exe = Join-Path $release 'kurasi.exe'
if (-not (Test-Path $exe)) { throw "kurasi.exe is missing at $exe" }
$out = Join-Path $repo 'release'
New-Item -ItemType Directory -Force $out | Out-Null

# Installer: keep the bundler's file and copy it to release\ with the name of CONTRACT_RUST.md section 6.
$setupSrc = Get-ChildItem (Join-Path $bundle 'nsis') -Filter "*$version*setup.exe" | Select-Object -First 1
$setupDst = $null
if ($setupSrc) {
    $setupDst = Join-Path $out "Kurasi-$version-Setup.exe"
    Copy-Item $setupSrc.FullName $setupDst -Force
    Write-Host "Installer: $setupDst ($([math]::Round($setupSrc.Length / 1MB, 1)) MB)"
}

# Portable folder and zip.
$stage = Join-Path $bundle "portable\Kurasi"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item $exe $stage
Copy-Item $pdfium $stage
Copy-Item (Join-Path $here 'installer\README.txt') (Join-Path $stage 'README.txt')
Copy-Item (Join-Path $repo 'LICENSE') (Join-Path $stage 'LICENSE.txt')
Copy-Item $thirdParty (Join-Path $stage 'THIRD_PARTY_LICENSES.txt')
# Licences of pdfium and its bundled libraries (resources\PDFIUM_SOURCE.md).
$lic = Join-Path $here 'resources\pdfium-licenses'
if (-not (Test-Path (Join-Path $lic 'pdfium.txt'))) { throw "the pdfium licence folder is missing at $lic" }
New-Item -ItemType Directory -Force (Join-Path $stage 'licenses') | Out-Null
Copy-Item $lic (Join-Path $stage 'licenses\pdfium') -Recurse
$uiSrc = Join-Path $here 'gen\ui'
if (-not (Test-Path (Join-Path $uiSrc 'index.html'))) { throw "the staged ui folder is missing at $uiSrc (build.rs creates it)" }
Copy-Item $uiSrc (Join-Path $stage 'ui') -Recurse
$marker = "Kurasi portable mode: settings and projects live in the folder data\ next to kurasi.exe.`r`nDelete this file to use %LOCALAPPDATA%\Kurasi instead.`r`n"
[IO.File]::WriteAllText((Join-Path $stage 'portable.txt'), $marker, [Text.UTF8Encoding]::new($false))
New-Item -ItemType Directory -Force (Join-Path $stage 'data') | Out-Null

$zip = Join-Path $out "Kurasi-$version-portable.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
# The zip is written with ZipArchive. It keeps the empty data\ folder, reads files that Compress-Archive
# reports as in use by another process, and writes entry names with forward slashes.
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::Open($zip, [IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($f in Get-ChildItem $stage -Recurse -File) {
        $name = $f.FullName.Substring($stage.Length + 1).Replace('\', '/')
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $f.FullName, $name, [IO.Compression.CompressionLevel]::Optimal) | Out-Null
    }
    $archive.CreateEntry('data/') | Out-Null
} finally {
    $archive.Dispose()
}
$zipItem = Get-Item $zip
Write-Host "Portable: $zip ($([math]::Round($zipItem.Length / 1MB, 1)) MB)"

# Checksums of this version's deliverables. LF line ends and no BOM, so `sha256sum -c SHA256SUMS.txt`
# checks them in Git Bash; Get-FileHash prints the same digits in capitals.
$sums = foreach ($f in @($setupDst, $zip)) {
    if ($f -and (Test-Path $f)) {
        '{0}  {1}' -f (Get-FileHash -LiteralPath $f -Algorithm SHA256).Hash.ToLowerInvariant(), (Split-Path -Leaf $f)
    }
}
$sumsFile = Join-Path $out 'SHA256SUMS.txt'
[IO.File]::WriteAllText($sumsFile, (($sums -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))
Write-Host "Checksums: $sumsFile"
