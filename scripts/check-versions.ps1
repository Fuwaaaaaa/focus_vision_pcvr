# scripts/check-versions.ps1
#
# Parses every version-bearing file in the repo and exits non-zero if any
# value disagrees. The canonical version is read from the top of the file
# (VERSION). All other files are compared against it.
#
# Sources (must all stay in sync):
#   VERSION                                  (4-component, e.g. 3.0.0.0)
#   rust/streaming-engine/Cargo.toml         (version = "3.0.0")
#   rust/companion-app/Cargo.toml            (version = "3.0.0")
#   rust/common/Cargo.toml                   (version = "3.0.0")
#   client/app/build.gradle.kts              (versionName = "3.0.0")
#   README.md                                (badge: version-3.0.0)
#
# Run locally:    pwsh ./scripts/check-versions.ps1
# CI invocation:  same — exit code is the gate signal.

$ErrorActionPreference = 'Stop'

function Get-RepoRoot {
    $here = Split-Path -Parent $PSCommandPath
    return (Resolve-Path (Join-Path $here '..')).Path
}

$root = Get-RepoRoot
$errors = @()

# 1. Canonical version (first line of VERSION, e.g. "3.0.0.0" → "3.0.0")
$versionFile = Join-Path $root 'VERSION'
if (-not (Test-Path $versionFile)) {
    Write-Host "ERROR: VERSION file missing at $versionFile"
    exit 1
}
$rawVersion = (Get-Content $versionFile -First 1).Trim()
$parts = $rawVersion -split '\.'
if ($parts.Count -lt 3) {
    Write-Host "ERROR: VERSION '$rawVersion' has fewer than 3 components"
    exit 1
}
$canonical = "$($parts[0]).$($parts[1]).$($parts[2])"
Write-Host "Canonical version (from VERSION): $canonical (raw: $rawVersion)"

function Assert-CargoVersion($path, $expected) {
    $full = Join-Path $root $path
    $line = Get-Content $full | Select-String -Pattern '^version\s*=' | Select-Object -First 1
    if (-not $line) {
        $script:errors += "$path : no version line found"
        return
    }
    if ($line -match 'version\s*=\s*"([^"]+)"') {
        $v = $Matches[1]
        if ($v -ne $expected) {
            $script:errors += "$path : version=$v (expected $expected)"
        } else {
            Write-Host "OK  $path  version=$v"
        }
    } else {
        $script:errors += "$path : version line did not parse: $line"
    }
}

Assert-CargoVersion 'rust/streaming-engine/Cargo.toml' $canonical
Assert-CargoVersion 'rust/companion-app/Cargo.toml'   $canonical
Assert-CargoVersion 'rust/common/Cargo.toml'          $canonical

# 4. client/app/build.gradle.kts versionName
$gradleFile = Join-Path $root 'client/app/build.gradle.kts'
$gradleLine = Get-Content $gradleFile | Select-String -Pattern 'versionName\s*=' | Select-Object -First 1
if ($gradleLine -match 'versionName\s*=\s*"([^"]+)"') {
    $v = $Matches[1]
    if ($v -ne $canonical) {
        $errors += "client/app/build.gradle.kts : versionName=$v (expected $canonical)"
    } else {
        Write-Host "OK  client/app/build.gradle.kts  versionName=$v"
    }
} else {
    $errors += "client/app/build.gradle.kts : versionName not found"
}

# 5. README badge
$readme = Join-Path $root 'README.md'
$badgeLine = Get-Content $readme | Select-String -Pattern 'badge/version-' | Select-Object -First 1
if ($badgeLine -match 'badge/version-([0-9]+\.[0-9]+\.[0-9]+)') {
    $v = $Matches[1]
    if ($v -ne $canonical) {
        $errors += "README.md : badge version-$v (expected $canonical)"
    } else {
        Write-Host "OK  README.md  badge=$v"
    }
} else {
    $errors += "README.md : version badge not found"
}

if ($errors.Count -gt 0) {
    Write-Host ""
    Write-Host "Version mismatches:"
    foreach ($e in $errors) { Write-Host "  - $e" }
    Write-Host ""
    Write-Host "Fix by editing the files above so every version reads $canonical."
    exit 1
}

Write-Host ""
Write-Host "All version sources agree on $canonical."
exit 0
