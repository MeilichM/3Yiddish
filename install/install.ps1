# 3Yiddish installer (Windows):
#   irm https://github.com/TripleU613/3Yiddish/releases/latest/download/install.ps1 | iex
$ErrorActionPreference = 'Stop'
$repo = 'TripleU613/3Yiddish'
$t = 'x86_64-pc-windows-msvc'
$base = "https://github.com/$repo/releases/latest/download"
$dir = Join-Path $env:LOCALAPPDATA '3yiddish\bin'
New-Item -ItemType Directory -Force $dir | Out-Null
$exe = Join-Path $dir '3yiddish.exe'
$tmp = "$exe.download"
Write-Host "Downloading 3Yiddish for $t..."
Invoke-WebRequest "$base/3yiddish-$t.exe" -OutFile $tmp -UseBasicParsing
$sums = (Invoke-WebRequest "$base/SHA256SUMS" -UseBasicParsing).Content
if ($sums -is [byte[]]) { $sums = [Text.Encoding]::UTF8.GetString($sums) }
$line = ($sums -split "`n") | Where-Object { $_ -match " 3yiddish-$t\.exe\s*$" } | Select-Object -First 1
$want = if ($line) { ($line -split ' ')[0].Trim() } else { '' }
$got = (Get-FileHash $tmp -Algorithm SHA256).Hash.ToLower()
if (-not $want -or $want -ne $got) { Remove-Item $tmp; throw 'Checksum mismatch; nothing was installed.' }
Move-Item -Force $tmp $exe
$path = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($path -split ';') -notcontains $dir) {
  [Environment]::SetEnvironmentVariable('Path', "$path;$dir", 'User')
  $env:Path += ";$dir"
}
Write-Host "Installed: $exe"
& $exe model pull
Write-Host 'Start it with: 3yiddish'
