@echo off
rem Double-click to pick guide(s), or drag-and-drop guide .md files onto this file.
rem Writes <Name>_portable.md next to each guide with every figure embedded.
setlocal
cd /d "%~dp0"
if "%~1"=="" (
  python "%~dp0embed_guide_images.py"
) else (
  python "%~dp0embed_guide_images.py" %*
)
echo.
pause
