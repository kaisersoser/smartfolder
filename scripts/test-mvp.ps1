param()

$ErrorActionPreference = "Stop"

$cargo = (Get-Command cargo -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -ErrorAction SilentlyContinue)
if (-not $cargo) {
    $fallback = Join-Path $HOME ".cargo\bin\cargo.exe"
    if (Test-Path $fallback) {
        $cargo = $fallback
    } else {
        throw "cargo was not found on PATH and no fallback was available at $fallback"
    }
}

Push-Location (Join-Path $PSScriptRoot "..")
try {
    & $cargo fmt --check
    & $cargo clippy --workspace --all-targets -- -D warnings
    & $cargo test --workspace
} finally {
    Pop-Location
}
