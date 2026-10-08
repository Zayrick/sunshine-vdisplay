# Verifies `vdisplay start --exclusive` disables physical displays and `stop` restores them.
# Falls back to `DisplaySwitch.exe /extend` if the layout isn't restored.
param(
    [string]$Exe = "$PSScriptRoot\..\target\release\vdisplay.exe",
    [int]$HoldSeconds = 5
)

Add-Type -AssemblyName System.Windows.Forms

function Get-Layout {
    [System.Windows.Forms.Screen]::AllScreens |
        Sort-Object DeviceName |
        ForEach-Object { "$($_.DeviceName) primary=$($_.Primary) $($_.Bounds)" }
}

# Screen.AllScreens is cached per process; query in a fresh process each time.
function Get-FreshLayout {
    powershell -NoProfile -Command "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Screen]::AllScreens | Sort-Object DeviceName | ForEach-Object { `$_.DeviceName + ' primary=' + `$_.Primary + ' ' + `$_.Bounds }"
}

$before = Get-FreshLayout
"== before"; $before

& $Exe start --exclusive --width 1920 --height 1080 --fps 60 --name ExclusiveTest
if ($LASTEXITCODE -ne 0) { "start failed ($LASTEXITCODE)"; exit 1 }
Start-Sleep -Seconds 2

$during = Get-FreshLayout
"== during"; $during
Start-Sleep -Seconds $HoldSeconds

& $Exe stop
Start-Sleep -Seconds 2

$after = Get-FreshLayout
"== after"; $after

$exclusiveOk = @($during).Count -eq 1
$restoreOk = (Compare-Object @($before) @($after)) -eq $null

if (-not $restoreOk) {
    "layout not restored, running DisplaySwitch /extend as fallback"
    DisplaySwitch.exe /extend
    Start-Sleep -Seconds 3
    "== after fallback"; Get-FreshLayout
}

"exclusive: $(if ($exclusiveOk) { 'OK' } else { 'FAILED' })"
"restore:   $(if ($restoreOk) { 'OK' } else { 'FAILED' })"
