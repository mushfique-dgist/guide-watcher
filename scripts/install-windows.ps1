<#
Run only after a successful fresh Tauri NSIS build. Never selects the newest file implicitly.
Example (from project root):
  .\scripts\install-windows.ps1 -InstallerPath '.\src-tauri\target\release\bundle\nsis\Guide Watcher_2.0.0_x64-setup.exe' -ExpectedBinaryPath '.\src-tauri\target\release\guide-watcher.exe'
Uses standard Tauri NSIS per-user registration, then verifies exact installed bytes.
https://v2.tauri.app/distribute/windows-installer/
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$InstallerPath,
    [Parameter(Mandatory)][string]$ExpectedBinaryPath,
    [switch]$RepairOnly
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'GuideWatcher.Install.ps1')
$installer = Get-Item -LiteralPath $InstallerPath
$binary = Get-Item -LiteralPath $ExpectedBinaryPath
if ($installer.PSIsContainer -or $installer.Name -notmatch '^Guide Watcher_.+_.*-setup\.exe$') { throw 'Specify the freshly built Guide Watcher NSIS setup executable.' }
if ($binary.PSIsContainer -or $binary.Name -ine 'guide-watcher.exe') { throw 'Specify the freshly built guide-watcher.exe.' }
Assert-GuideWatcherDesktopBinary -Path $binary.FullName
$expectedHash = Get-GuideWatcherNsisHash -Path $binary.FullName
$installDir = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'Guide Watcher'
$installedExe = Join-Path $installDir 'guide-watcher.exe'
if (!$RepairOnly) {
    if (@(Get-Process -Name 'guide-watcher' -ErrorAction SilentlyContinue).Count) {
        throw 'Close Guide Watcher, including its tray icon, before installing. Running guide jobs will not be stopped automatically.'
    }
    # NSIS requires /D last and unquoted, including paths with spaces.
    $process = Start-Process -FilePath $installer.FullName -ArgumentList "/S /D=$installDir" -WindowStyle Hidden -PassThru
    if (!$process.WaitForExit(180000)) { throw 'Installation is still running after three minutes. Inspect the installer before retrying; it was not terminated.' }
    if ($process.ExitCode -ne 0) { throw "Guide Watcher installer failed with exit code $($process.ExitCode)." }
}
if (!(Test-Path -LiteralPath $installedExe -PathType Leaf)) { throw "Installation did not create $installedExe." }
if ((Get-FileHash -LiteralPath $installedExe -Algorithm SHA256).Hash -ne $expectedHash) {
    throw 'Installed executable does not match the requested build. Shortcuts were not repaired. Rebuild and rerun the installer.'
}
$result = Repair-GuideWatcherShortcut -InstalledExe $installedExe -LegacyBuildExe $binary.FullName
$result
Write-Output "Verified installed SHA256: $expectedHash"
Write-Output 'Start menu entry is ready. Windows Search may take time to refresh; no search service or index settings were changed.'
