# winrec
轻量命令行录制工具

# 查看显示器列表
``winrec --list``

# 正斜杠 / 反斜杠都能正常识别，自动创建不存在的目录

``winrec -o D:/captures/2026/demo1.mp4 -f 30``
``winrec -o D:\captures\2026\demo2.mp4 -f 60 -m 1``

# 定时录 15 秒后自动停止保存
``winrec -o test.mp4 -t 15``

# winrec

轻量级 Windows 命令行录屏工具。基于 Windows Graphics Capture（WGC）GPU 抓屏 + Media Foundation HEVC 硬件编码，录制完成后自动生成含 MD5/SHA256 的校验文件（`.sum`）。

## 环境要求

- Windows 10 1903 及以上（Windows Graphics Capture API）
- 支持 HEVC 编码的 GPU 或系统编码器
- Rust 工具链（仅构建时需要）

## 构建与安装

```powershell
cargo build --release          # 产物在 target\release\winrec.exe
cargo install --path .         # 安装到 ~/.cargo/bin
```

## 快速上手

```powershell
winrec                                  # 主显示器，30fps，8 Mbps，Ctrl+C 停止
winrec -o demo.mp4 -f 15 -b 6           # 指定输出、15fps、6 Mbps
winrec -m 2 -t 60                       # 第二台显示器，录 60 秒自动停止
winrec --list                           # 查看所有显示器
winrec --no-sum                         # 跳过 .sum 校验文件
```

## 命令行参数

| 参数 | 短写 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--output` | `-o` | `output.mp4` | 输出路径，支持 `/` 和 `\`；无后缀自动补 `.mp4`，父目录不存在自动创建 |
| `--fps` | `-f` | `30` | 目标帧率（最小 1）；系统支持时同时限制捕获帧率上限 |
| `--bitrate` | `-b` | `8` | 目标视频码率（Mbps，最小 1），文件大小约等于码率 × 时长 |
| `--monitor` | `-m` | `0` | 显示器编号，从 1 开始；`0` 表示主显示器，可用 `--list` 查看 |
| `--duration` | `-t` | `0` | 自动录制时长（秒）；`0` 表示直到按 Ctrl+C 停止 |
| `--no-sum` | | 关闭 | 录制完成后跳过生成 `.sum` 校验文件 |
| `--list` | | | 列出所有可用显示器并退出 |

## 输出文件

- 视频：MP4 容器，HEVC（H.265）编码，按显示器原始分辨率录制。
- 校验文件：与视频同名的 `.sum` 文本文件，例如 `demo.mp4` 生成 `demo.mp4.sum`：

```text
文件名：demo.mp4
录制开始时间：2026-11-26 15:30:00
录制结束时间：2026-11-26 15:31:00
MD5:e10adc3949ba59abbe56e057f20f883e
SHA256:b17ef6d19c7a5b1e9b2d5a3f...
```

## 工作原理与体积调优

- 抓屏与编码全程走 GPU（WGC 纹理直接送 Media Foundation 编码器），CPU 占用低。
- 帧时间戳取自真实捕获时刻：画面静止时不产帧，回放时长仍然准确。
- 编码采用**固定目标码率**（非 ffmpeg `-crf` 那种恒定质量模式），文件大小可预估：
  - 6 Mbps ≈ 45 MB/分钟，8 Mbps ≈ 60 MB/分钟，15 Mbps ≈ 112 MB/分钟。
  - 录屏内容以静态桌面为主，6–10 Mbps 观感差异很小；需要更小体积就调低 `-b`。
- 帧率节流（把捕获帧率限制在 `-f` 以内）依赖系统的 `MinUpdateInterval` 属性（Windows 11 24H2+）；程序启动时自动探测，不支持则静默跳过，仅编码器按标称帧率工作。

## 与 ffmpeg gdigrab 的差异

| | winrec | ffmpeg gdigrab |
| --- | --- | --- |
| 抓屏 | WGC，GPU 零拷贝 | GDI，CPU 逐帧拷贝 |
| 编码 | HEVC，固定目标码率 | 任意编码器（如 NVENC `-crf` 恒定质量） |
| 体积 | 码率 × 时长，可预估 | CRF 模式下静止画面几乎不耗码率，通常更小 |
| 校验 | 自动生成 MD5/SHA256 `.sum` 文件 | 无 |
