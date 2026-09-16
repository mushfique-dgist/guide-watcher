Set-StrictMode -Version Latest

function Repair-GuideWatcherShortcut {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$InstalledExe,
        [string]$ProgramsPath = [Environment]::GetFolderPath('Programs'),
        [string]$LegacyBuildExe
    )
    $ErrorActionPreference = 'Stop'
    $exe = [IO.Path]::GetFullPath($InstalledExe)
    if ([IO.Path]::GetFileName($exe) -ine 'guide-watcher.exe' -or !(Test-Path -LiteralPath $exe -PathType Leaf)) {
        throw 'The installed guide-watcher.exe is missing. Run the current NSIS installer first.'
    }
    $programs = [IO.Path]::GetFullPath($ProgramsPath)
    $folder = Join-Path $programs 'Guide Watcher'
    $canonical = Join-Path $folder 'Guide Watcher.lnk'
    $shell = New-Object -ComObject WScript.Shell
    try {
        $ownedTargets = @($exe)
        if ($LegacyBuildExe) { $ownedTargets += [IO.Path]::GetFullPath($LegacyBuildExe) }
        # Inspect all candidates before any write; never claim a same-name foreign shortcut.
        $duplicates = @()
        if (Test-Path -LiteralPath $programs) {
            foreach ($item in Get-ChildItem -LiteralPath $programs -Filter '*.lnk' -File -Recurse) {
                $link = $shell.CreateShortcut($item.FullName)
                $owned = ($ownedTargets -contains $link.TargetPath) -and [string]::IsNullOrEmpty($link.Arguments)
                if ($item.FullName -ieq $canonical -and !$owned) {
                    throw "The Guide Watcher shortcut belongs to another target: $($link.TargetPath). It was left unchanged."
                }
                # Startup links have separate intent and must never be removed as duplicates.
                $relative = $item.FullName.Substring($programs.TrimEnd('\').Length + 1)
                if ($owned -and $item.FullName -ine $canonical -and $relative -notlike 'Startup\*') {
                    $duplicates += $item.FullName
                }
            }
        }
        New-Item -ItemType Directory -Path $folder -Force | Out-Null
        $shortcut = $shell.CreateShortcut($canonical)
        $changed = !(Test-Path -LiteralPath $canonical) -or $shortcut.TargetPath -ine $exe -or $shortcut.WorkingDirectory -ine [IO.Path]::GetDirectoryName($exe) -or $shortcut.IconLocation -ine "$exe,0" -or $shortcut.Description -ne 'Generate and review study guides'
        if ($changed) {
            $shortcut.TargetPath = $exe
            $shortcut.Arguments = ''
            $shortcut.WorkingDirectory = [IO.Path]::GetDirectoryName($exe)
            $shortcut.IconLocation = "$exe,0"
            $shortcut.Description = 'Generate and review study guides'
            $shortcut.Save()
        }
        $check = $shell.CreateShortcut($canonical)
        if ($check.TargetPath -ine $exe -or !(Test-Path -LiteralPath $check.TargetPath -PathType Leaf)) {
            throw 'Shortcut verification failed; duplicate shortcuts were retained.'
        }
        foreach ($duplicate in $duplicates) { Remove-Item -LiteralPath $duplicate }
        [pscustomobject]@{ Shortcut = $canonical; Executable = $exe; Changed = $changed; RemovedDuplicates = $duplicates.Count }
    } finally {
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
    }
}

function Assert-GuideWatcherDesktopBinary {
    param([Parameter(Mandatory)][string]$Path)
    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -lt 128 -or [BitConverter]::ToUInt16($bytes, 0) -ne 0x5a4d) { throw 'Expected a Windows desktop executable, not an empty or malformed file.' }
    $offset = [BitConverter]::ToInt32($bytes, 0x3c)
    if ($offset -lt 64 -or $offset -gt $bytes.Length - 94 -or [BitConverter]::ToUInt32($bytes, $offset) -ne 0x4550) { throw 'The selected executable has an invalid PE header.' }
    if ([BitConverter]::ToUInt16($bytes, $offset + 92) -ne 2) { throw 'The selected executable is a console program. Build the Guide Watcher desktop target before installing.' }
}

# Tauri patches UNK to NSS for NSIS, then restores the build executable.
# Match the expected packaged bytes in memory; never modify either executable.
# https://docs.rs/crate/tauri-bundler/latest/source/src/bundle.rs
function Get-GuideWatcherNsisHash {
    param([Parameter(Mandatory)][string]$Path)
    $bytes = [IO.File]::ReadAllBytes($Path)
    $text = [Text.Encoding]::ASCII.GetString($bytes)
    $token = '__TAURI_BUNDLE_TYPE_VAR_UNK'
    $index = $text.IndexOf($token, [StringComparison]::Ordinal)
    if ($index -lt 0 -or $text.IndexOf($token, $index + 1, [StringComparison]::Ordinal) -ge 0) {
        throw 'Expected exactly one unbundled Tauri marker. Rebuild with the supported bundler before installing.'
    }
    [Text.Encoding]::ASCII.GetBytes('__TAURI_BUNDLE_TYPE_VAR_NSS').CopyTo($bytes, $index)
    return ([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($bytes))).Replace('-', '')
}
