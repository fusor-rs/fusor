# Installs the fusor CLI on Windows:
#
#   irm https://fusor.build/install.ps1 | iex
#
# Set $env:FUSOR_VERSION = "v0.1.8" to install a specific release, and
# $env:FUSOR_INSTALL to choose the directory (default ~\.fusor). Binaries go in
# its bin\, which is added to your user PATH.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repository = 'fusor-rs/fusor'
# Overridable so the release workflow can test this script against the archives
# it has just built, before they are published.
$downloadBase = if ($env:FUSOR_DOWNLOAD_BASE) { $env:FUSOR_DOWNLOAD_BASE } else { "https://github.com/$repository/releases/download" }
$installDir = if ($env:FUSOR_INSTALL) { $env:FUSOR_INSTALL } else { Join-Path $HOME '.fusor' }
$version = $env:FUSOR_VERSION

# Windows on Arm runs the x64 build through its built-in emulation.
$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($architecture -ne 'X64' -and $architecture -ne 'Arm64') {
  throw "fusor has no Windows build for $architecture"
}
$target = 'x86_64-pc-windows-msvc'

if (-not $version) {
  $version = (Invoke-RestMethod "https://api.github.com/repos/$repository/releases/latest").tag_name
  if (-not $version) { throw 'could not find the latest fusor release' }
}
if (-not $version.StartsWith('v')) { $version = "v$version" }
$number = $version.Substring(1)

$archive = "fusor-$number-$target.zip"
$url = "$downloadBase/$version/$archive"
$bin = Join-Path $installDir 'bin'
$temporary = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $temporary | Out-Null
try {
  Write-Host "Downloading fusor $version for $target"
  Invoke-WebRequest $url -OutFile (Join-Path $temporary $archive) -UseBasicParsing
  Invoke-WebRequest "$url.sha256" -OutFile (Join-Path $temporary "$archive.sha256") -UseBasicParsing

  $expected = ((Get-Content (Join-Path $temporary "$archive.sha256") -Raw).Trim() -split '\s+')[0].ToLowerInvariant()
  $actual = (Get-FileHash (Join-Path $temporary $archive) -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($expected -ne $actual) {
    throw "checksum mismatch for $archive; the download is corrupt or was tampered with"
  }

  Expand-Archive (Join-Path $temporary $archive) -DestinationPath $temporary
  New-Item -ItemType Directory -Force -Path $bin | Out-Null
  Move-Item -Force (Join-Path $temporary "fusor-$number-$target\fusor.exe") (Join-Path $bin 'fusor.exe')
  # Releases up to 0.1.0 also installed cargo-fusor; don't leave a stale copy.
  Remove-Item -Force (Join-Path $bin 'cargo-fusor.exe') -ErrorAction SilentlyContinue
} finally {
  Remove-Item -Recurse -Force $temporary -ErrorAction SilentlyContinue
}

$installed = & (Join-Path $bin 'fusor.exe') --version
Write-Host "Installed $installed to $bin"

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not (($userPath -split ';') -contains $bin)) {
  [Environment]::SetEnvironmentVariable('Path', "$bin;$userPath", 'User')
  Write-Host "Added $bin to your PATH. Open a new terminal to use fusor."
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
  Write-Host ''
  Write-Host 'fusor builds your application with Rust, which is not installed.'
  Write-Host 'Install it from https://rustup.rs, then run: fusor new my-app'
}
