@echo off
setlocal
cd /d "%~dp0"
where cargo >nul 2>&1
if errorlevel 1 (
  echo Install Rust from https://rustup.rs/ and try again.
  pause
  exit /b 1
)
cargo run --release
if errorlevel 1 (
  echo The build or test failed.
  pause
  exit /b 1
)
echo.
echo Results saved in input_timing_results.csv
pause
