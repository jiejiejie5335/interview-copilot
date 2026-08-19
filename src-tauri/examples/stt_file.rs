use std::{env, time::Instant};

use interview_copilot_lib::stt::{
    refine_samples_with_model, transcribe_samples, validate_local_installation, RefinementModel,
};

fn main() -> anyhow::Result<()> {
    let mut arguments = env::args().skip(1);
    let audio_path = arguments
        .next()
        .ok_or_else(|| anyhow::anyhow!("usage: cargo run --example stt_file -- AUDIO.wav"))?;
    let refinement_model = match arguments.next().as_deref() {
        Some("qwen") => RefinementModel::Qwen3,
        _ => RefinementModel::SenseVoice,
    };
    validate_local_installation(refinement_model)?;
    let mut reader = hound::WavReader::open(audio_path)?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != 16_000
        || spec.sample_format != hound::SampleFormat::Int
        || spec.bits_per_sample != 16
    {
        anyhow::bail!("测试文件必须是 16 kHz、单声道、16-bit PCM WAV");
    }
    let samples = reader
        .samples::<i16>()
        .map(|sample| sample.map(|value| value as f32 / i16::MAX as f32))
        .collect::<Result<Vec<_>, _>>()?;

    let started = Instant::now();
    let fast = transcribe_samples(&samples, 16_000)?;
    println!("FAST_TRANSCRIPT={fast}");
    println!("FAST_INFERENCE_MS={}", started.elapsed().as_millis());

    let refinement_started = Instant::now();
    let final_text = refine_samples_with_model(&samples, 16_000, &fast, refinement_model)?;
    println!("FINAL_TRANSCRIPT={final_text}");
    println!(
        "FINAL_INFERENCE_MS={}",
        refinement_started.elapsed().as_millis()
    );
    Ok(())
}
