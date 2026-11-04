use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use anyhow::{anyhow, Context, Result};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, FromSample, SampleFormat, SizedSample, Stream, StreamConfig,
};
use serde::Deserialize;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
    sync::mpsc,
    time::{self, Duration},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StreamHeader {
    sample_rate: u32,
    channels: u8,
    format: String,
}

async fn run_helper_stream<F, R>(
    stop: Arc<AtomicBool>,
    mut on_samples: F,
    on_ready: R,
) -> Result<()>
where
    F: FnMut(&[f32], u32) + Send + 'static,
    R: FnOnce(u32) + Send + 'static,
{
    let mut command = Command::new(env!("INTERVIEW_AUDIO_TAP_BIN"));
    let mut child = command
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("无法启动 macOS 音频 helper")?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("无法读取音频 helper 输出"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("无法读取音频 helper 错误输出"))?;
    let mut reader = BufReader::new(stdout);
    let mut header_line = String::new();
    reader
        .read_line(&mut header_line)
        .await
        .context("无法读取音频流格式")?;
    let header: StreamHeader = serde_json::from_str(header_line.trim())
        .with_context(|| format!("音频 helper 返回了无效格式：{header_line}"))?;
    if header.channels != 1 || header.format != "f32le" || header.sample_rate == 0 {
        return Err(anyhow!("不支持的音频流格式"));
    }
    on_ready(header.sample_rate);

    let mut poll_stop = time::interval(Duration::from_millis(80));
    let mut bytes = vec![0_u8; 32 * 1024];
    let mut carry = Vec::<u8>::new();

    loop {
        tokio::select! {
            _ = poll_stop.tick() => {
                if stop.load(Ordering::Acquire) {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    return Ok(());
                }
            }
            read = reader.read(&mut bytes) => {
                let count = read.context("读取 Core Audio PCM 失败")?;
                if count == 0 {
                    let status = child.wait().await.context("等待音频 helper 退出失败")?;
                    let mut detail = String::new();
                    let _ = stderr.read_to_string(&mut detail).await;
                    let detail = detail.trim();
                    return Err(if detail.is_empty() {
                        anyhow!("Core Audio helper 意外退出：{status}")
                    } else {
                        anyhow!("Core Audio helper 意外退出：{detail}")
                    });
                }

                carry.extend_from_slice(&bytes[..count]);
                let complete = carry.len() / 4 * 4;
                let samples = carry[..complete]
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect::<Vec<_>>();
                carry.drain(..complete);
                if !samples.is_empty() {
                    on_samples(&samples, header.sample_rate);
                }
            }
        }
    }
}

/// Captures the macOS system output through a Core Audio Process Tap.
pub async fn run_pcm_stream<F, R>(stop: Arc<AtomicBool>, on_samples: F, on_ready: R) -> Result<()>
where
    F: FnMut(&[f32], u32) + Send + 'static,
    R: FnOnce(u32) + Send + 'static,
{
    run_helper_stream(stop, on_samples, on_ready).await
}

fn build_input_stream<T, F>(
    device: &Device,
    config: StreamConfig,
    on_samples: Arc<Mutex<F>>,
    error_sender: mpsc::UnboundedSender<String>,
) -> Result<Stream>
where
    T: SizedSample + Copy + Send + 'static,
    f32: FromSample<T>,
    F: FnMut(&[f32], u32) + Send + 'static,
{
    let channels = usize::from(config.channels.max(1));
    let sample_rate = config.sample_rate;
    device
        .build_input_stream::<T, _, _>(
            config,
            move |input, _| {
                let mut mono = Vec::with_capacity(input.len() / channels);
                for frame in input.chunks(channels) {
                    let sum = frame
                        .iter()
                        .copied()
                        .map(<f32 as FromSample<T>>::from_sample_)
                        .sum::<f32>();
                    mono.push(sum / frame.len() as f32);
                }
                if let Ok(mut callback) = on_samples.try_lock() {
                    callback(&mono, sample_rate);
                }
            },
            move |error| {
                let _ = error_sender.send(error.to_string());
            },
            None,
        )
        .context("无法创建默认麦克风输入流")
}

/// Captures the default macOS microphone in the Tauri process. Keeping the
/// stream in the host process gives macOS a stable permission identity across
/// Cargo rebuilds and lets CPAL report Zoom/Bluetooth route invalidations.
pub async fn run_microphone_pcm_stream<F, R>(
    stop: Arc<AtomicBool>,
    on_samples: F,
    on_ready: R,
) -> Result<()>
where
    F: FnMut(&[f32], u32) + Send + 'static,
    R: FnOnce(u32) + Send + 'static,
{
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("没有可用的默认麦克风"))?;
    let device_id = device.id().context("无法读取默认麦克风标识")?;
    let supported = device
        .default_input_config()
        .context("无法读取默认麦克风格式")?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let sample_rate = config.sample_rate;
    let on_samples = Arc::new(Mutex::new(on_samples));
    let (error_sender, mut error_receiver) = mpsc::unbounded_channel::<String>();
    let stream = match sample_format {
        SampleFormat::F32 => {
            build_input_stream::<f32, _>(&device, config, on_samples.clone(), error_sender.clone())?
        }
        SampleFormat::I16 => {
            build_input_stream::<i16, _>(&device, config, on_samples.clone(), error_sender.clone())?
        }
        SampleFormat::U16 => {
            build_input_stream::<u16, _>(&device, config, on_samples.clone(), error_sender.clone())?
        }
        format => return Err(anyhow!("不支持的麦克风采样格式：{format}")),
    };
    stream.play().context("无法启动默认麦克风输入流")?;
    on_ready(sample_rate);

    let mut poll = time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            error = error_receiver.recv() => {
                return Err(anyhow!(
                    "麦克风输入流中断：{}",
                    error.unwrap_or_else(|| "音频设备连接已关闭".to_string())
                ));
            }
            _ = poll.tick() => {
                if stop.load(Ordering::Acquire) {
                    drop(stream);
                    return Ok(());
                }
                let current_id = host
                    .default_input_device()
                    .and_then(|current| current.id().ok());
                if current_id.as_ref() != Some(&device_id) {
                    return Err(anyhow!("默认麦克风已切换，需要重新连接"));
                }
            }
        }
    }
}
