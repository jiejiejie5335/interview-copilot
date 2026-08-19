use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
    time::{self, Duration},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StreamHeader {
    sample_rate: u32,
    channels: u8,
    format: String,
}

/// Runs the native Core Audio helper. The helper writes one JSON header line,
/// followed by raw mono Float32 little-endian PCM until it is stopped.
pub async fn run_pcm_stream<F>(stop: Arc<AtomicBool>, mut on_samples: F) -> Result<()>
where
    F: FnMut(&[f32], u32) + Send + 'static,
{
    let mut child = Command::new(env!("INTERVIEW_AUDIO_TAP_BIN"))
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("无法启动 Core Audio Tap helper")?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("无法读取音频 helper 输出"))?;
    let mut stderr = child.stderr.take().ok_or_else(|| anyhow!("无法读取音频 helper 错误输出"))?;
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

