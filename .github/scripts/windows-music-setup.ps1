# End-to-end `grimoire music-setup` on a clean Windows machine.
#
# Run 1 is what a new user sees: answer "y", let Grimoire download the
# installer, install, start the app once, enable the API Server plugin, and
# start it again. It stops at pairing, because pairing puts an Allow/Deny
# dialog in front of a person and there is no person here.
#
# So between runs the script does what clicking Allow does — adds Grimoire to
# the plugin's authorised clients — and run 2 pairs for real and saves a token,
# which is then used against the API.

$ErrorActionPreference = 'Stop'
$grimoire = (Resolve-Path './target/release/grimoire.exe').Path
$tmp = $env:RUNNER_TEMP

function Invoke-Setup([string]$name, [string]$stdin, [string]$until, [int]$minutes) {
    $in = Join-Path $tmp "$name.in"
    $out = Join-Path $tmp "$name.out"
    Set-Content -Path $in -Value $stdin -NoNewline
    $p = Start-Process $grimoire -ArgumentList 'music-setup' -NoNewWindow -PassThru `
        -RedirectStandardInput $in -RedirectStandardOutput $out -RedirectStandardError "$out.err"
    $deadline = (Get-Date).AddMinutes($minutes)
    while ((Get-Date) -lt $deadline -and -not $p.HasExited) {
        if ((Test-Path $out) -and (Select-String -Path $out -Pattern $until -Quiet)) { break }
        Start-Sleep -Seconds 2
    }
    if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
    Write-Host "----- $name stdout"
    Get-Content $out | Write-Host
    Write-Host "----- $name stderr"
    Get-Content "$out.err" | Write-Host
    return (Get-Content $out -Raw)
}

function Assert-Has([string]$text, [string]$pattern, [string]$why) {
    if ($text -notmatch $pattern) { throw "$why (expected output matching '$pattern')" }
}

# ── Run 1 ────────────────────────────────────────────────────────────────────
$one = Invoke-Setup 'run1' "y`n" 'Pairing' 20
Assert-Has $one 'not installed' 'a clean machine should have no YouTube Music'
Assert-Has $one 'nothing to choose' 'Windows should skip the CPU question'
Assert-Has $one 'Web-Setup' 'Windows should download the web installer'
Assert-Has $one 'Installed to .*YouTube Music\.exe' 'the installer should leave YouTube Music.exe in place'
Assert-Has $one 'turned on, bound to 127\.0\.0\.1:26538' 'the plugin should be switched on in the new config'
Assert-Has $one 'API server responding' 'the relaunched app should answer on 26538'

$exe = Join-Path $env:LOCALAPPDATA 'Programs/youtube-music/YouTube Music.exe'
if (-not (Test-Path $exe)) { throw "no $exe" }

$configPath = Join-Path $env:APPDATA 'YouTube Music/config.json'
$config = Get-Content $configPath -Raw | ConvertFrom-Json -AsHashtable
$api = $config.plugins.'api-server'
Write-Host "api-server config: $($api | ConvertTo-Json -Compress -Depth 5)"
if (-not $api.enabled -or $api.hostname -ne '127.0.0.1' -or $api.port -ne 26538) {
    throw 'the API Server plugin config is not what setup writes'
}

# ── Stand in for the person clicking Allow ───────────────────────────────────
# Close the app first (it would rewrite its config on the way out, and the
# unanswered pairing dialog from run 1 goes with it). Run 2 then has to start
# it again itself.
Get-Process -Name 'YouTube Music' -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 3
$config = Get-Content $configPath -Raw | ConvertFrom-Json -AsHashtable
$api = $config.plugins.'api-server'
$api.authorizedClients = @('grimoiretui')
$config | ConvertTo-Json -Depth 100 | Set-Content -Path $configPath -Encoding utf8NoBOM

# ── Run 2 ────────────────────────────────────────────────────────────────────
$two = Invoke-Setup 'run2' '' 'Done\.' 5
Assert-Has $two 'found .*YouTube Music\.exe' 'run 2 should find the installed app'
Assert-Has $two 'already on' 'run 2 should leave the plugin alone'
Assert-Has $two 'Paired\.' 'run 2 should pair'

$musicToml = Get-Content (Join-Path $HOME '.config/grimoire/music.toml') -Raw
if ($musicToml -notmatch 'token\s*=\s*"([^"]+)"') { throw "no token saved:`n$musicToml" }
$token = $Matches[1]

$res = Invoke-WebRequest -Uri 'http://127.0.0.1:26538/api/v1/song' -SkipHttpErrorCheck `
    -Headers @{ Authorization = "Bearer $token" }
Write-Host "GET /api/v1/song with the saved token -> $($res.StatusCode)"
if ($res.StatusCode -in 401, 403) { throw 'the saved token was refused' }

$anon = Invoke-WebRequest -Uri 'http://127.0.0.1:26538/api/v1/song' -SkipHttpErrorCheck
Write-Host "GET /api/v1/song without a token -> $($anon.StatusCode)"
if ($anon.StatusCode -ne 401) { throw 'the API should refuse requests without the token' }

Write-Host 'music-setup works end to end on Windows.'
