# 3Yiddish installer (Windows):
#   irm https://github.com/TripleU613/3Yiddish/releases/latest/download/install.ps1 | iex
# There are two builds: one that uses the graphics card (Vulkan) and a regular one.
# The installer picks; $env:THREE_YIDDISH_VARIANT = 'cpu' or 'gpu' forces the choice.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$repo = 'TripleU613/3Yiddish'
$t = 'x86_64-pc-windows-msvc'
$base = if ($env:THREE_YIDDISH_BASE) { $env:THREE_YIDDISH_BASE } else { "https://github.com/$repo/releases/latest/download" }
$dir = if ($env:THREE_YIDDISH_BIN) { $env:THREE_YIDDISH_BIN } else { Join-Path $env:LOCALAPPDATA '3yiddish\bin' }
New-Item -ItemType Directory -Force $dir | Out-Null
$exe = Join-Path $dir '3yiddish.exe'
# The download keeps an .exe name so it can be run once before it replaces anything.
$tmp = Join-Path $dir '3yiddish-download.exe'
$sums = $null
$why = ''

# Download one build, check it against SHA256SUMS and make sure it runs here.
function Get-Build($name) {
  $script:why = 'The download failed'
  try {
    Invoke-WebRequest "$base/$name" -OutFile $tmp -UseBasicParsing
    if (-not $script:sums) {
      $s = (Invoke-WebRequest "$base/SHA256SUMS" -UseBasicParsing).Content
      if ($s -is [byte[]]) { $s = [Text.Encoding]::UTF8.GetString($s) }
      $script:sums = $s
    }
  } catch { return $false }
  $script:why = 'Checksum mismatch'
  # sha256sum writes "hash  name", or "hash *name" when it ran on Windows.
  $pattern = '[ *]' + [regex]::Escape($name) + '\s*$'
  $line = ($script:sums -split "`n") | Where-Object { $_ -match $pattern } | Select-Object -First 1
  $want = if ($line) { ($line -split ' ')[0].Trim() } else { '' }
  $got = (Get-FileHash $tmp -Algorithm SHA256).Hash.ToLower()
  if (-not $want -or $want -ne $got) { return $false }
  $script:why = 'This build does not run on this system'
  try {
    $global:LASTEXITCODE = 1
    & $tmp --version | Out-Null
    return ($LASTEXITCODE -eq 0)
  } catch { return $false }
}

# A build that cannot start should fail quietly instead of opening a Windows error box.
try {
  $k = Add-Type -Name Kernel -Namespace ThreeYiddish -PassThru -MemberDefinition '[DllImport("kernel32.dll")] public static extern uint SetErrorMode(uint mode);'
  $k::SetErrorMode(0x8003) | Out-Null
} catch { }

$gpu = $false
switch -Regex ("$env:THREE_YIDDISH_VARIANT") {
  '^(gpu|vulkan)$' { $gpu = $true; break }
  '^cpu$' { break }
  '^(auto)?$' {
    # The graphics-card build needs vulkan-1.dll, which comes with the graphics driver.
    if (Test-Path (Join-Path $env:WINDIR 'System32\vulkan-1.dll')) {
      $gpu = $true
      Write-Host 'Using the graphics card (GPU) build.'
    } else {
      Write-Host 'No graphics driver found; using the regular build.'
    }
    break
  }
  default { throw 'THREE_YIDDISH_VARIANT must be cpu or gpu.' }
}
Write-Host "Downloading 3Yiddish for $t..."
$ok = $false
if ($gpu) {
  $ok = Get-Build "3yiddish-$t-vulkan.exe"
  if (-not $ok) { Write-Host 'The graphics card (GPU) build did not work here; using the regular build.' }
}
if (-not $ok) { $ok = Get-Build "3yiddish-$t.exe" }
if (-not $ok) {
  Remove-Item $tmp -ErrorAction SilentlyContinue
  throw "$why; nothing was installed."
}
Move-Item -Force $tmp $exe
$path = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($path -split ';') -notcontains $dir) {
  [Environment]::SetEnvironmentVariable('Path', "$path;$dir", 'User')
  $env:Path += ";$dir"
}
Write-Host "Installed: $exe"
if ($env:THREE_YIDDISH_SKIP_MODEL) { return }
& $exe model pull
Write-Host 'Start it with: 3yiddish'
