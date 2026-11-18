[CmdletBinding()]
param(
    [ValidateSet('x64', 'arm64')]
    [string]$Architecture = 'x64'
)

$ErrorActionPreference = 'Stop'
$target = @{ x64 = 'x86_64-pc-windows-msvc'; arm64 = 'aarch64-pc-windows-msvc' }[$Architecture]
$outputDirectory = if ($Architecture -eq 'x64') { 'dist' } else { 'dist\arm64' }

Push-Location -LiteralPath $PSScriptRoot
try {
    rustup target add $target
    if ($LASTEXITCODE -ne 0) { throw "Could not install the Rust target $target." }

    cargo build --locked --release --target $target --target-dir target
    if ($LASTEXITCODE -ne 0) { throw 'The Windows release build failed.' }

    $null = New-Item -ItemType Directory -Force -Path $outputDirectory
    $exe = Join-Path $outputDirectory 'FisoGen.exe'
    Copy-Item -LiteralPath "target\$target\release\FisoGen.exe" -Destination $exe -Force
    Write-Host "Built $(Join-Path $PSScriptRoot $exe)"
}
finally {
    Pop-Location
}
