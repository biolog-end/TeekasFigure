@echo off
chcp 65001 >nul

cargo run --release --example prepare_shapes

echo.
echo ===================================================
echo input_shapes.
echo ===================================================
pause