use std::{
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use interview_copilot_lib::{
    audio::macos::run_pcm_stream,
    stt::{refine_samples, transcribe_samples, validate_installation, SpeechSegmenter},
};
use tokio::{process::Command, sync::mpsc, time};

fn write_debug_wav(samples: &[f32]) -> anyhow::Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create("/private/tmp/interview-captured.wav", spec)?;
    for sample in samples {
        writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    validate_installation()?;
    let audio_path = env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: cargo run --example stt_smoke -- AUDIO_FILE"))?;
    let stop = Arc::new(AtomicBool::new(false));
    let capture_stop = stop.clone();
    let (sender, mut receiver) = mpsc::channel::<Vec<f32>>(2);

    let capture = tokio::spawn(async move {
        let mut segmenter = SpeechSegmenter::default();
        run_pcm_stream(capture_stop, move |samples, sample_rate| {
            for segment in segmenter.push(samples, sample_rate) {
                let _ = sender.try_send(segment);
            }
        })
        .await
    });

    time::sleep(Duration::from_secs(1)).await;
    let playback = Command::new("afplay").arg(audio_path).status().await?;
    if !playback.success() {
        anyhow::bail!("afplay failed: {playback}");
    }

    let segment = time::timeout(Duration::from_secs(8), receiver.recv())
        .await?
        .ok_or_else(|| anyhow::anyhow!("capture ended without a speech segment"))?;
    let rms = (segment.iter().map(|value| value * value).sum::<f32>() / segment.len() as f32).sqrt();
    let peak = segment.iter().fold(0.0_f32, |peak, value| peak.max(value.abs()));
    println!(
        "CAPTURED={} samples ({:.2}s), rms={rms:.4}, peak={peak:.4}",
        segment.len(),
        segment.len() as f32 / 16_000.0
    );
    write_debug_wav(&segment)?;
    stop.store(true, Ordering::Release);
    let started = Instant::now();
    let text = transcribe_samples(&segment, 16_000)?;
    println!("FAST_TRANSCRIPT={text}");
    println!("FAST_INFERENCE_MS={}", started.elapsed().as_millis());
    let refinement_started = Instant::now();
    let refined = refine_samples(&segment, 16_000, &text)?;
    println!("FINAL_TRANSCRIPT={refined}");
    println!(
        "FINAL_INFERENCE_MS={}",
        refinement_started.elapsed().as_millis()
    );
    capture.await??;
    Ok(())
}
