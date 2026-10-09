# Windows 构建

标准 MSVC 构建需要 Visual Studio C++ Build Tools。缺少 MSVC 链接器时，可使用 `stable-x86_64-pc-windows-gnu` 和项目 `.tools/` 内的 LLVM-MinGW；下面的脚本只为当前进程设置工具链与 dlltool，不修改系统 PATH。

GNU 构建入口：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\cargo-local.ps1 test --locked
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\cargo-local.ps1 clippy --locked --all-targets -- -D warnings
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\cargo-local.ps1 build --locked --release
.\target\release\fgo-rs.exe run scripts/inspect_rider.rhai --replay tests/fixtures/step2.png
```

`cargo-local.ps1` 只为该进程设置构建环境并转发 Cargo 参数。标准 MSVC 环境或 macOS 不需要它，直接使用 Cargo 即可。

用于验证的本地下载来源：

- Rust 官方 rustup：`rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal --component rustfmt --component clippy`
- [LLVM-MinGW 20260908](https://github.com/mstorsjo/llvm-mingw/releases/tag/20260908)，文件 `llvm-mingw-20260908-msvcrt-x86_64.zip`。
- SHA-256：`341cc9786b54956467ac19a02fdb00749307eac133da56108f643e4189db810b`。
- 解压至 `.tools/`，得到 `.tools/llvm-mingw-20260908-msvcrt-x86_64/bin/`；该目录被 Git 忽略。

这些是开发工具，不是应用运行时依赖，不应打入应用发布包。
