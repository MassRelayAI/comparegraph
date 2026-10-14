# Installs the comparegraph release binary for Windows x64 and checks it against the release's SHA256SUMS.
#   irm https://raw.githubusercontent.com/massrelay-ai/comparegraph/main/scripts/install.ps1 | iex
# Environment: COMPAREGRAPH_VERSION (a tag; default latest), COMPAREGRAPH_INSTALL_DIR (default %LOCALAPPDATA%\Programs\comparegraph),
# COMPAREGRAPH_BASE_URL (where the assets are read from; default the GitHub release).
$ErrorActionPreference = 'Stop'
$repo = 'massrelay-ai/comparegraph'
if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
  Write-Error "comparegraph: no prebuilt binary for this architecture; build from source: cargo install --locked --git https://github.com/$repo comparegraph"
}
$asset = 'comparegraph-win32-x64.exe'
$version = if ($env:COMPAREGRAPH_VERSION) { $env:COMPAREGRAPH_VERSION } else { 'latest' }
$dir = if ($env:COMPAREGRAPH_INSTALL_DIR) { $env:COMPAREGRAPH_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\comparegraph' }
$base = if ($env:COMPAREGRAPH_BASE_URL) { $env:COMPAREGRAPH_BASE_URL }
        elseif ($version -eq 'latest') { "https://github.com/$repo/releases/latest/download" }
        else { "https://github.com/$repo/releases/download/$version" }

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("comparegraph-" + [System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  Write-Host "comparegraph: fetching $asset ($version)"
  Invoke-WebRequest -Uri "$base/$asset" -OutFile (Join-Path $tmp $asset)
  Invoke-WebRequest -Uri "$base/SHA256SUMS" -OutFile (Join-Path $tmp 'SHA256SUMS')
  $line = Get-Content (Join-Path $tmp 'SHA256SUMS') | Where-Object { $_ -match "\s\*?$([regex]::Escape($asset))$" } | Select-Object -First 1
  if (-not $line) { throw "comparegraph: $asset is not listed in SHA256SUMS" }
  $expected = ($line -split '\s+')[0].ToLower()
  $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $tmp $asset)).Hash.ToLower()
  if ($actual -ne $expected) { throw "comparegraph: checksum mismatch for $asset (expected $expected, got $actual); nothing was installed" }
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  Move-Item -Force (Join-Path $tmp $asset) (Join-Path $dir 'comparegraph.exe')
  Write-Host "comparegraph: installed $dir\comparegraph.exe"
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  if (($userPath -split ';') -notcontains $dir) {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$dir", 'User')
    Write-Host "comparegraph: added $dir to your user PATH; open a new terminal"
  }
  Write-Host "comparegraph: next, wire your agents: comparegraph setup --apply   (then: comparegraph doctor)"
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
