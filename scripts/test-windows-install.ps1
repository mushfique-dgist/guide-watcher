$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'GuideWatcher.Install.ps1')
function Assert($condition, $message) { if (!$condition) { throw $message } }
function Assert-Throws($action, $message) { $threw = $false; try { & $action | Out-Null } catch { $threw = $true }; Assert $threw $message }
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('Guide Watcher install tests ' + [guid]::NewGuid())
$programs = Join-Path $testRoot 'Start Menu Programs'
$exe = Join-Path $testRoot 'Installed App\guide-watcher.exe'
$legacy = Join-Path $testRoot 'Old Build\guide-watcher.exe'
$shell = New-Object -ComObject WScript.Shell
function New-Link($path, $target, $arguments = '') {
    New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($path)) -Force | Out-Null
    $link = $shell.CreateShortcut($path); $link.TargetPath = $target; $link.Arguments = $arguments; $link.Save()
}
try {
    Assert-Throws { Repair-GuideWatcherShortcut -InstalledExe $exe -ProgramsPath $programs } 'Missing executable must fail'
    Assert (!(Test-Path -LiteralPath $programs)) 'Missing executable modified Start menu'
    New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($exe)) -Force | Out-Null
    [IO.File]::WriteAllBytes($exe, [byte[]](77,90))
    $first = Repair-GuideWatcherShortcut -InstalledExe $exe -ProgramsPath $programs
    Assert $first.Changed 'Missing shortcut was not created'
    Assert ($shell.CreateShortcut($first.Shortcut).TargetPath -eq $exe) 'Space-containing target did not round trip'
    $hash = (Get-FileHash -LiteralPath $first.Shortcut).Hash
    $stamp = (Get-Item -LiteralPath $first.Shortcut).LastWriteTimeUtc
    $second = Repair-GuideWatcherShortcut -InstalledExe $exe -ProgramsPath $programs
    Assert (!$second.Changed) 'Repeated repair was not idempotent'
    Assert ((Get-Item -LiteralPath $first.Shortcut).LastWriteTimeUtc -eq $stamp) 'Repeated repair rewrote shortcut'
    Assert ((Get-FileHash -LiteralPath $first.Shortcut).Hash -eq $hash) 'Repeated repair changed bytes'
    New-Link $first.Shortcut $legacy
    $oldLink = Join-Path $programs 'Old Folder\Old Guide Watcher.lnk'
    New-Link $oldLink $legacy
    $dup = Join-Path $programs 'Duplicate.lnk'; New-Link $dup $exe
    $foreign = Join-Path $programs 'Other App\Guide Watcher.lnk'; New-Link $foreign 'C:\Other\unrelated.exe'
    $foreignHash = (Get-FileHash -LiteralPath $foreign).Hash
    $argsLink = Join-Path $programs 'Custom invocation.lnk'; New-Link $argsLink $exe '--custom'
    $startup = Join-Path $programs 'Startup\Guide Watcher.lnk'; New-Link $startup $legacy
    $fixed = Repair-GuideWatcherShortcut -InstalledExe $exe -ProgramsPath $programs -LegacyBuildExe $legacy
    Assert ($fixed.RemovedDuplicates -eq 2) 'Owned duplicate cleanup incorrect'
    Assert (!(Test-Path -LiteralPath $oldLink) -and !(Test-Path -LiteralPath $dup)) 'Owned duplicates remain'
    Assert ((Get-FileHash -LiteralPath $foreign).Hash -eq $foreignHash) 'Foreign registration changed'
    Assert (Test-Path -LiteralPath $argsLink) 'Custom invocation removed'
    Assert (Test-Path -LiteralPath $startup) 'Startup registration removed'
    New-Link $first.Shortcut 'C:\Other\unrelated.exe'
    $collisionHash = (Get-FileHash -LiteralPath $first.Shortcut).Hash
    Assert-Throws { Repair-GuideWatcherShortcut -InstalledExe $exe -ProgramsPath $programs -LegacyBuildExe $legacy } 'Foreign canonical collision must fail'
    Assert ((Get-FileHash -LiteralPath $first.Shortcut).Hash -eq $collisionHash) 'Canonical collision overwritten'
    # A directory occupying the destination fails without deleting owned duplicates.
    Remove-Item -LiteralPath $first.Shortcut
    New-Item -ItemType Directory -Path $first.Shortcut | Out-Null
    New-Link $dup $exe
    Assert-Throws { Repair-GuideWatcherShortcut -InstalledExe $exe -ProgramsPath $programs } 'Unwritable destination must fail'
    Assert (Test-Path -LiteralPath $dup) 'Partial failure deleted duplicate'
    $installer = Join-Path $testRoot 'Guide Watcher_2.0.0_x64-setup.exe'
    [IO.File]::WriteAllBytes($installer, [byte[]](77,90))
    (Get-Item -LiteralPath $installer).LastWriteTimeUtc = (Get-Item -LiteralPath $exe).LastWriteTimeUtc.AddMinutes(-1)
    Assert-Throws { & (Join-Path $PSScriptRoot 'install-windows.ps1') -InstallerPath $installer -ExpectedBinaryPath $exe } 'Malformed build must fail before execution'
    Assert-Throws { & (Join-Path $PSScriptRoot 'install-windows.ps1') -InstallerPath $exe -ExpectedBinaryPath $exe } 'Wrong installer must fail before execution'
    $pe = New-Object byte[] 256
    [BitConverter]::GetBytes([uint16]0x5a4d).CopyTo($pe, 0)
    [BitConverter]::GetBytes([int32]64).CopyTo($pe, 0x3c)
    [BitConverter]::GetBytes([uint32]0x4550).CopyTo($pe, 64)
    [BitConverter]::GetBytes([uint16]2).CopyTo($pe, 156)
    [IO.File]::WriteAllBytes($exe, $pe)
    Assert-GuideWatcherDesktopBinary -Path $exe
    [BitConverter]::GetBytes([uint16]3).CopyTo($pe, 156)
    [IO.File]::WriteAllBytes($exe, $pe)
    Assert-Throws { Assert-GuideWatcherDesktopBinary -Path $exe } 'Console binary must be rejected'
    [BitConverter]::GetBytes([int32]2147483647).CopyTo($pe, 0x3c)
    [IO.File]::WriteAllBytes($exe, $pe)
    Assert-Throws { Assert-GuideWatcherDesktopBinary -Path $exe } 'Out-of-range PE header must be rejected'
    $markerFile = Join-Path $testRoot 'marker.bin'
    [IO.File]::WriteAllBytes($markerFile, [Text.Encoding]::ASCII.GetBytes('before__TAURI_BUNDLE_TYPE_VAR_UNKafter'))
    $beforeHash = (Get-FileHash -LiteralPath $markerFile).Hash
    $actualHash = Get-GuideWatcherNsisHash -Path $markerFile
    $expectedBytes = [Text.Encoding]::ASCII.GetBytes('before__TAURI_BUNDLE_TYPE_VAR_NSSafter')
    Assert ($actualHash -eq ([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($expectedBytes))).Replace('-', '')) 'Packaged marker hash mismatch'
    Assert ((Get-FileHash -LiteralPath $markerFile).Hash -eq $beforeHash) 'Verifier modified source binary'
    [IO.File]::WriteAllText($markerFile, '__TAURI_BUNDLE_TYPE_VAR_UNK__TAURI_BUNDLE_TYPE_VAR_UNK')
    Assert-Throws { Get-GuideWatcherNsisHash -Path $markerFile } 'Duplicate markers must fail'
    [IO.File]::WriteAllText($markerFile, 'no marker')
    Assert-Throws { Get-GuideWatcherNsisHash -Path $markerFile } 'Missing marker must fail'
    Write-Output 'PASS: missing executable/menu, spaces, exact shortcut target, byte/time idempotency, stale links, duplicates, foreign/near-match targets, arguments, startup preservation, collision, partial failure, stale/wrong installer preflight.'
} finally {
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
    $resolved = [IO.Path]::GetFullPath($testRoot)
    $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    if (!$resolved.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($resolved) -notlike 'Guide Watcher install tests *') { throw 'Unsafe test cleanup path' }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
