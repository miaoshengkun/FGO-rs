$CargoArgs = $args
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
$rustup = Join-Path $cargoBin 'rustup.exe'
$toolchainRoot = (& $rustup run stable-x86_64-pc-windows-gnu rustc --print sysroot)
if ($LASTEXITCODE -ne 0) { throw 'Install stable-x86_64-pc-windows-gnu first; see docs/local-windows.md' }
$gnuTools = Join-Path $toolchainRoot 'lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained'
$llvmTools = Join-Path $projectRoot '.tools\llvm-mingw-20260908-msvcrt-x86_64\bin'
if (-not (Test-Path -LiteralPath $llvmTools)) { throw 'Local LLVM-MinGW not found; see docs/local-windows.md' }
$previousPath = $env:PATH
$previousLinker = $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER
try {
    $env:PATH = "$cargoBin;$llvmTools;$gnuTools;$previousPath"
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = Join-Path $gnuTools 'x86_64-w64-mingw32-gcc.exe'
    Push-Location -LiteralPath $projectRoot
    try {
        & (Join-Path $cargoBin 'cargo.exe') '+stable-x86_64-pc-windows-gnu' @CargoArgs
        $result = $LASTEXITCODE
    } finally { Pop-Location }
} finally {
    $env:PATH = $previousPath
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = $previousLinker
}
exit $result
