# scripts/sign-windows.ps1
#
# Authenticode-signs a single .exe / .dll / .msi using a PFX certificate
# delivered via two environment variables:
#
#   WINDOWS_PFX_BASE64    base64-encoded contents of a .pfx file
#   WINDOWS_PFX_PASSWORD  password protecting the .pfx
#
# Designed for CI: pulls the cert from GitHub secrets, decodes to a
# temp .pfx, runs signtool, deletes the temp .pfx in a finally block so
# the cert never lingers on disk. Errors are fatal (we'd rather fail
# the build than ship a half-signed artifact).
#
# Local invocation:
#   $env:WINDOWS_PFX_BASE64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes("cert.pfx"))
#   $env:WINDOWS_PFX_PASSWORD = "..."
#   pwsh ./scripts/sign-windows.ps1 -FilePath path/to/file.exe
#
# Caller responsibility: gate this script on the secrets being present.
# It does NOT silently no-op — calling without env vars set is treated
# as a configuration error.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$FilePath,

    # Timestamp server. DigiCert is the de-facto default; switch to
    # http://timestamp.sectigo.com if DigiCert is unreachable.
    [string]$TimestampUrl = 'http://timestamp.digicert.com',

    # Description embedded in the signature ("Subject" in cert dialogs).
    [string]$Description = 'Focus Vision PCVR'
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $FilePath)) {
    throw "file to sign does not exist: $FilePath"
}
if (-not $env:WINDOWS_PFX_BASE64) {
    throw 'WINDOWS_PFX_BASE64 not set — caller must gate on this env var'
}
if (-not $env:WINDOWS_PFX_PASSWORD) {
    throw 'WINDOWS_PFX_PASSWORD not set — required alongside WINDOWS_PFX_BASE64'
}

# Locate signtool. On windows-latest runners the Windows SDK ships it under
# C:\Program Files (x86)\Windows Kits\10\bin\<ver>\x64\signtool.exe; the
# specific version directory drifts, so glob.
$signtool = (Get-Command signtool.exe -ErrorAction SilentlyContinue)?.Source
if (-not $signtool) {
    $candidates = Get-ChildItem -Path 'C:\Program Files (x86)\Windows Kits\10\bin' -Recurse `
        -Filter signtool.exe -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match 'x64\\signtool\.exe$' } |
        Sort-Object FullName -Descending
    if ($candidates) {
        $signtool = $candidates[0].FullName
    }
}
if (-not $signtool) {
    throw 'signtool.exe not found — install Windows SDK or add to PATH'
}
Write-Host "signtool: $signtool"

# Stage the cert in a temp file. Random name + immediate cleanup in
# finally so it's not left on the runner if signtool fails mid-flight.
$pfxPath = Join-Path ([System.IO.Path]::GetTempPath()) "fvp-sign-$([System.Guid]::NewGuid().ToString('N')).pfx"
try {
    [System.IO.File]::WriteAllBytes(
        $pfxPath,
        [System.Convert]::FromBase64String($env:WINDOWS_PFX_BASE64)
    )

    Write-Host "Signing: $FilePath"
    & $signtool sign `
        /f $pfxPath `
        /p $env:WINDOWS_PFX_PASSWORD `
        /fd sha256 `
        /tr $TimestampUrl `
        /td sha256 `
        /d $Description `
        /v `
        $FilePath
    if ($LASTEXITCODE -ne 0) {
        throw "signtool exited $LASTEXITCODE"
    }

    # Verify the signature embedded successfully.
    & $signtool verify /pa /v $FilePath
    if ($LASTEXITCODE -ne 0) {
        throw "signtool verify failed (exit $LASTEXITCODE)"
    }
    Write-Host "OK: $FilePath signed and verified"
}
finally {
    if (Test-Path $pfxPath) {
        Remove-Item $pfxPath -Force -ErrorAction SilentlyContinue
    }
}
