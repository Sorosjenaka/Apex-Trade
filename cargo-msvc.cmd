@echo off
rem Wrapper: cargo dengan MSVC Build Tools 18 (vcvars64) — fix masalah dlltool/gcc di toolchain GNU.
rem Pakai: cargo-msvc.cmd <args cargo>, contoh: cargo-msvc.cmd test --workspace
rem Target folder WAJIB satu: target-msvc (jangan buat folder target baru).
call "C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
if errorlevel 1 (
  echo [cargo-msvc] gagal memuat vcvars64.bat
  exit /b 1
)
set RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc
set "CARGO_TARGET_DIR=%~dp0target-msvc"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cargo %*
