@echo off
rem Local build: VS env + VS clang-cl first on PATH (WinLibs clang-cl would otherwise win).
rem Optional env: KYTY_BUILD_DIR (default _Build/windows), KYTY_EXTRA_CMAKE (extra configure args).
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul || exit /b 1
set "PATH=%VCINSTALLDIR%Tools\Llvm\x64\bin;C:\VulkanSDK\1.4.363.0\Bin;%PATH%"
if "%KYTY_BUILD_DIR%"=="" set "KYTY_BUILD_DIR=_Build/windows"
cd /d "%~dp0.."
if not exist %KYTY_BUILD_DIR%\build.ninja (
  cmake -S . -B %KYTY_BUILD_DIR% -G Ninja -DCMAKE_BUILD_TYPE=Release ^
    -DCMAKE_C_COMPILER=clang-cl -DCMAKE_CXX_COMPILER=clang-cl ^
    -DCMAKE_PREFIX_PATH="C:/Qt/6.10.3/msvc2022_64" %KYTY_EXTRA_CMAKE% || exit /b 1
)
cmake --build %KYTY_BUILD_DIR% %* || exit /b 1
