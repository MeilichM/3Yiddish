# Make a throwaway app home for testing the desktop app, so the real one is never touched.
# The installed model is hard-linked in (no 550 MB copy; the test home must be on the same
# drive as %APPDATA%), and a second, hand-placed copy of it is added so the model switch shows.
# A recording from the real home is copied to audio\sample.wav for transcription tests.
param([string]$TestHome = "$env:TEMP\3y-desktop-test")
$ErrorActionPreference = 'Stop'

$real = if ($env:THREE_YIDDISH_REAL_HOME) { $env:THREE_YIDDISH_REAL_HOME } else { "$env:APPDATA\3yiddish\data" }
$models = "$real\state\models"
if (-not (Test-Path "$models\current")) { throw "No installed model under $models. Open the app once and download it." }
$version = (Get-Content "$models\current" -Raw).Trim()
$src = "$models\$version"

$dst = "$TestHome\state\models"
New-Item -ItemType Directory -Force "$dst\$version", "$dst\hand-copy", "$TestHome\audio", "$TestHome\shots" | Out-Null
foreach ($dir in $version, 'hand-copy') {
    foreach ($f in Get-ChildItem $src -File | Where-Object Name -ne 'model.json') {
        if (-not (Test-Path "$dst\$dir\$($f.Name)")) {
            New-Item -ItemType HardLink -Path "$dst\$dir\$($f.Name)" -Target $f.FullName | Out-Null
        }
    }
}
Copy-Item "$src\model.json" "$dst\$version\model.json" -Force
$m = Get-Content "$src\model.json" -Raw | ConvertFrom-Json
$m.version = 'hand-copy'
$m | Add-Member local $true -Force
$m | Add-Member label 'Hand copy' -Force
[IO.File]::WriteAllText("$dst\hand-copy\model.json", ($m | ConvertTo-Json -Depth 6))
[IO.File]::WriteAllText("$dst\current", $version)
Remove-Item "$dst\previous" -ErrorAction SilentlyContinue

$sample = Get-ChildItem "$real\state\contributions\audio", "$real\state\desktop\recordings" -Filter *.wav -ErrorAction SilentlyContinue | Select-Object -First 1
if ($sample) { Copy-Item $sample.FullName "$TestHome\audio\sample.wav" -Force }

"Test home: $TestHome"
"Models:    $version (active), hand-copy (label 'Hand copy', local)"
"Sample:    $(if ($sample) { "$TestHome\audio\sample.wav" } else { 'none found; bring your own .wav' })"
