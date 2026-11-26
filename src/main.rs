use chrono::Local;
use clap::Parser;
use md5::{Digest as Md5Digest, Md5};
use sha2::{Digest as Sha2Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DrawBorderSettings, Settings,
};

#[derive(Parser, Debug)]
#[command(name = "winrec", about = "轻量 Windows 命令行录屏工具")]
struct Args {
    /// 输出文件路径（支持正斜杠 / 和反斜杠 \，默认 output.mp4）
    #[arg(short, long, default_value = "output.mp4")]
    output: String,

    /// 目标帧率（如 30 或 60）
    #[arg(short, long, default_value_t = 30)]
    fps: u32,

    /// 显示器编号（从 1 开始；不填或 0 表示主显示器；传入 --list 可查看）
    #[arg(short, long, default_value_t = 0)]
    monitor: usize,

    /// 自动录制时长（秒，0 表示直到按 Ctrl+C 停止）
    #[arg(short = 't', long, default_value_t = 0)]
    duration: u64,

    /// 录制完成后跳过生成同名 .sum 校验文件
    #[arg(long)]
    no_sum: bool,

    /// 列出所有可用显示器并退出
    #[arg(long)]
    list: bool,
}

/// 规范化路径，兼容正斜杠 `/` 与反斜杠 `\`，并确保父目录存在
fn prepare_output_path(raw: &str) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let normalized = raw.replace('/', "\\");
    let mut path = PathBuf::from(normalized);

    if path.extension().is_none() {
        path.set_extension("mp4");
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    Ok(path)
}

fn select_monitor(index: usize) -> Result<Monitor, Box<dyn std::error::Error + Send + Sync>> {
    if index == 0 {
        return Ok(Monitor::primary()?);
    }

    let monitors = Monitor::enumerate()?;
    if monitors.is_empty() {
        return Err("未检测到可用显示器".into());
    }
    if index > monitors.len() {
        return Err(format!(
            "显示器编号 {} 超出范围（当前共 {} 个显示器，请用 --list 查看）",
            index,
            monitors.len()
        )
        .into());
    }

    Ok(monitors[index - 1].clone())
}

/// 生成同名的 `.sum` 路径，例如 `a.mp4` -> `a.mp4.sum`
fn sum_file_path(video_path: &Path) -> PathBuf {
    let mut os_str = video_path.as_os_str().to_os_string();
    os_str.push(".sum");
    PathBuf::from(os_str)
}

/// 流式计算文件的 MD5 与 SHA256
fn calc_hashes(file_path: &Path) -> Result<(String, String), Box<dyn std::error::Error + Send + Sync>> {
    let file = File::open(file_path)?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut buf = [0u8; 64 * 1024];

    let mut md5_hasher = Md5::new();
    let mut sha256_hasher = Sha256::new();

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        md5_hasher.update(chunk);
        sha256_hasher.update(chunk);
    }

    let md5_hex = format!("{:x}", md5_hasher.finalize());
    let sha256_hex = format!("{:x}", sha256_hasher.finalize());

    Ok((md5_hex, sha256_hex))
}

/// 写入同名 `.sum` 文件
fn write_sum_file(
    video_path: &Path,
    start_time_str: &str,
    end_time_str: &str,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let (md5_hex, sha256_hex) = calc_hashes(video_path)?;
    let sum_path = sum_file_path(video_path);

    let file_name = video_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("output.mp4");

    let content = format!(
        "文件名：{}\n录制开始时间：{}\n录制结束时间：{}\nMD5:{}\nSHA256:{}\n",
        file_name, start_time_str, end_time_str, md5_hex, sha256_hex,
    );

    fs::write(&sum_path, content)?;
    Ok(sum_path)
}

struct CaptureFlags {
    output_path: PathBuf,
    fps: u32,
    stop_flag: Arc<AtomicBool>,
    start_instant: Instant,
    max_duration: Option<Duration>,
}

struct Recorder {
    encoder: Option<VideoEncoder>,
    flags: CaptureFlags,
    finished: bool,
}

impl Recorder {
    fn finalize(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;

        if let Some(encoder) = self.encoder.take() {
            encoder.finish()?;
        }
        Ok(())
    }
}

impl GraphicsCaptureApiHandler for Recorder {
    type Flags = CaptureFlags;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let flags = ctx.flags;
        let width = ctx.item.width()?;
        let height = ctx.item.height()?;

        let out_str = flags
            .output_path
            .to_str()
            .ok_or("输出路径包含无效的 UTF-8 字符")?;

        let video_settings = VideoSettingsBuilder::new(width, height).frame_rate(flags.fps);

        let encoder = VideoEncoder::new(
            video_settings,
            AudioSettingsBuilder::default().disabled(true),
            ContainerSettingsBuilder::default(),
            out_str,
        )?;

        Ok(Self {
            encoder: Some(encoder),
            flags,
            finished: false,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let time_up = self
            .flags
            .max_duration
            .map(|d| self.flags.start_instant.elapsed() >= d)
            .unwrap_or(false);

        if self.flags.stop_flag.load(Ordering::Relaxed) || time_up {
            self.finalize()?;
            capture_control.stop();
            return Ok(());
        }

        if let Some(encoder) = self.encoder.as_mut() {
            encoder.send_frame(frame)?;
        }

        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        self.finalize()?;
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();

    if args.list {
        let monitors = Monitor::enumerate()?;
        for (i, m) in monitors.iter().enumerate() {
            println!(
                "[{}] {} ({}x{})",
                i + 1,
                m.name().unwrap_or_else(|_| "Unknown".to_string()),
                m.width().unwrap_or(0),
                m.height().unwrap_or(0)
            );
        }
        return Ok(());
    }

    let output_path = prepare_output_path(&args.output)?;
    let target_monitor = select_monitor(args.monitor)?;

    let stop_flag = Arc::new(AtomicBool::new(false));
    let flag_clone = stop_flag.clone();
    ctrlc::set_handler(move || {
        flag_clone.store(true, Ordering::Relaxed);
    })?;

    let max_duration = if args.duration > 0 {
        Some(Duration::from_secs(args.duration))
    } else {
        None
    };

    let start_time_str = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let start_instant = Instant::now();

    let flags = CaptureFlags {
        output_path: output_path.clone(),
        fps: args.fps,
        stop_flag,
        start_instant,
        max_duration,
    };

    let settings = Settings::new(
        target_monitor,
        CursorCaptureSettings::Default,
        DrawBorderSettings::WithoutBorder,
        ColorFormat::Rgba8,
        flags,
    );

    println!(
        "开始录制 -> {} | 帧率: {} fps | 按 Ctrl+C 停止并保存",
        output_path.display(),
        args.fps
    );
    io::stdout().flush()?;

    Recorder::start(settings)?;
    let end_time_str = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    println!("已完成并保存: {}", output_path.display());

    if !args.no_sum {
        let sum_path = write_sum_file(&output_path, &start_time_str, &end_time_str)?;
        println!("已生成校验文件: {}", sum_path.display());
    }

    Ok(())
}
