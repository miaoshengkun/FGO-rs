# 本机 Windows 构建

本机 Rust 已安装于 `%USERPROFILE%\.cargo\bin`，但默认 MSVC 工具链缺少 C++ 链接器。
为完成首版验证，新增了 `stable-x86_64-pc-windows-gnu` 工具链，未更改默认工具链或系统 PATH。
GNU 工具链自带的 dlltool 缺少可调用的汇编器，因此使用项目 `.tools/` 内的 LLVM-MinGW 提供 dlltool。

本机直接使用：

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
