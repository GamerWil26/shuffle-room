param([string]$Target = "x86_64-pc-windows-msvc")
$ErrorActionPreference = "Stop"
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    cargo build --release --locked --target $Target
    if ($LASTEXITCODE -ne 0) { throw "Rust build failed" }
    $stage = "dist/shuffle-room-windows-x64-preview"
    New-Item -ItemType Directory -Force "$stage/bin" | Out-Null
    Copy-Item "target/$Target/release/shuffle-room.exe" $stage
    Copy-Item README.md,TESTING.md,LICENSE,THIRD_PARTY.md $stage
    Copy-Item "packaging/Start Shuffle Room.cmd" $stage
    Copy-Item "packaging/bin-README.txt" "$stage/bin/README.txt"
    python scripts/collect-notices.py --target $Target --out "$stage/third-party-notices"
    if ($LASTEXITCODE -ne 0) { throw "Notice collection failed" }
    Compress-Archive -Path $stage -DestinationPath "dist/shuffle-room-windows-x64-preview.zip" -Force
    Get-FileHash "dist/shuffle-room-windows-x64-preview.zip" -Algorithm SHA256
} finally { Pop-Location }
