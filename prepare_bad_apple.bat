@echo off
chcp 65001 >nul

REM Подготовка клипа Bad Apple в набор силуэтов-кистей (raw_shapes/).
REM Можно передать путь к видео и опции, например:
REM   prepare_bad_apple.bat "bad apple addon\Bad Apple!!.mp4" --interval 2 --max-size 512
cargo run --release --example prepare_bad_apple -- %*

echo.
echo ===================================================
echo Готово. Силуэты сохранены в raw_shapes/.
echo Для запуска поставьте use_original_colors = true в settings.toml
echo и положите клип в input_media/.
echo ===================================================
pause
