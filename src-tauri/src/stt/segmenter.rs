use std::collections::VecDeque;

const TARGET_RATE: u32 = 16_000;
const FRAME_SAMPLES: usize = 320;
const SPEECH_THRESHOLD: f32 = 0.008;
const PRE_ROLL_FRAMES: usize = 12;
const START_SPEECH_FRAMES: usize = 4;
const END_SILENCE_FRAMES: usize = 75;
const TRAILING_SILENCE_FRAMES: usize = 5;
const MIN_VOICED_FRAMES: usize = 15;
const MIN_SEGMENT_SAMPLES: usize = 12_800;
const MAX_SEGMENT_SAMPLES: usize = 16_000 * 14;

#[derive(Default)]
struct LinearResampler {
    source_rate: u32,
    previous: Option<f32>,
    position: f64,
}

impl LinearResampler {
    fn process(&mut self, input: &[f32], source_rate: u32) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        if source_rate == TARGET_RATE {
            return input.to_vec();
        }
        if self.source_rate != source_rate {
            self.source_rate = source_rate;
            self.previous = None;
            self.position = 0.0;
        }

        let previous = self.previous.unwrap_or(input[0]);
        let mut source = Vec::with_capacity(input.len() + 1);
        source.push(previous);
        source.extend_from_slice(input);
        let step = source_rate as f64 / TARGET_RATE as f64;
        let mut output = Vec::with_capacity((input.len() as f64 / step).ceil() as usize + 1);
        while self.position + 1.0 < source.len() as f64 {
            let index = self.position.floor() as usize;
            let fraction = (self.position - index as f64) as f32;
            output.push(source[index] + (source[index + 1] - source[index]) * fraction);
            self.position += step;
        }
        self.position -= (source.len() - 1) as f64;
        self.previous = input.last().copied();
        output
    }
}

pub struct SpeechSegmenter {
    resampler: LinearResampler,
    pending: Vec<f32>,
    pre_roll: VecDeque<Vec<f32>>,
    speech: Vec<f32>,
    in_speech: bool,
    voiced_run: usize,
    voiced_frames: usize,
    silence_frames: usize,
}

impl Default for SpeechSegmenter {
    fn default() -> Self {
        Self {
            resampler: LinearResampler::default(),
            pending: Vec::new(),
            pre_roll: VecDeque::new(),
            speech: Vec::new(),
            in_speech: false,
            voiced_run: 0,
            voiced_frames: 0,
            silence_frames: 0,
        }
    }
}

impl SpeechSegmenter {
    pub fn push(&mut self, samples: &[f32], sample_rate: u32) -> Vec<Vec<f32>> {
        self.pending.extend(self.resampler.process(samples, sample_rate));
        let mut completed = Vec::new();
        while self.pending.len() >= FRAME_SAMPLES {
            let frame = self.pending.drain(..FRAME_SAMPLES).collect::<Vec<_>>();
            if let Some(segment) = self.push_frame(frame) {
                completed.push(segment);
            }
        }
        completed
    }

    fn push_frame(&mut self, frame: Vec<f32>) -> Option<Vec<f32>> {
        let rms = (frame.iter().map(|value| value * value).sum::<f32>() / frame.len() as f32).sqrt();
        let voiced = rms >= SPEECH_THRESHOLD;

        if !self.in_speech {
            self.pre_roll.push_back(frame);
            while self.pre_roll.len() > PRE_ROLL_FRAMES {
                self.pre_roll.pop_front();
            }
            self.voiced_run = if voiced { self.voiced_run + 1 } else { 0 };
            if self.voiced_run >= START_SPEECH_FRAMES {
                self.in_speech = true;
                self.voiced_frames = self.voiced_run;
                self.silence_frames = 0;
                self.speech = self.pre_roll.drain(..).flatten().collect();
            }
            return None;
        }

        self.speech.extend_from_slice(&frame);
        if voiced {
            self.voiced_frames += 1;
        }
        self.silence_frames = if voiced { 0 } else { self.silence_frames + 1 };
        if self.silence_frames >= END_SILENCE_FRAMES || self.speech.len() >= MAX_SEGMENT_SAMPLES {
            return self.finish_segment();
        }
        None
    }

    fn finish_segment(&mut self) -> Option<Vec<f32>> {
        let voiced_frames = self.voiced_frames;
        let trailing_silence_frames = self.silence_frames;
        self.in_speech = false;
        self.voiced_run = 0;
        self.voiced_frames = 0;
        self.silence_frames = 0;
        self.pre_roll.clear();
        let mut segment = std::mem::take(&mut self.speech);
        // Only remove silence that was actually observed. The previous version
        // also cut 600 ms of real speech whenever the 14 s limit was reached.
        let trailing_silence = trailing_silence_frames
            .saturating_sub(TRAILING_SILENCE_FRAMES)
            * FRAME_SAMPLES;
        if segment.len() > trailing_silence + MIN_SEGMENT_SAMPLES {
            segment.truncate(segment.len() - trailing_silence);
        }
        (voiced_frames >= MIN_VOICED_FRAMES && segment.len() >= MIN_SEGMENT_SAMPLES)
            .then_some(segment)
    }
}

#[cfg(test)]
mod tests {
    use super::SpeechSegmenter;

    #[test]
    fn emits_speech_after_trailing_silence() {
        let mut segmenter = SpeechSegmenter::default();
        let mut output = Vec::new();
        output.extend(segmenter.push(&vec![0.1; 16_000], 16_000));
        output.extend(segmenter.push(&vec![0.0; 32_000], 16_000));
        assert_eq!(output.len(), 1);
        assert!(output[0].len() >= 16_000);
    }

    #[test]
    fn ignores_silence() {
        let mut segmenter = SpeechSegmenter::default();
        assert!(segmenter.push(&vec![0.0; 32_000], 16_000).is_empty());
    }

    #[test]
    fn ignores_short_echo_bursts() {
        let mut segmenter = SpeechSegmenter::default();
        let mut output = Vec::new();
        output.extend(segmenter.push(&vec![0.02; 3_200], 16_000));
        output.extend(segmenter.push(&vec![0.0; 16_000], 16_000));
        assert!(output.is_empty());
    }

    #[test]
    fn does_not_trim_real_speech_at_max_duration() {
        let mut segmenter = SpeechSegmenter::default();
        let output = segmenter.push(&vec![0.1; 16_000 * 15], 16_000);
        assert_eq!(output.len(), 1);
        assert!(output[0].len() >= 16_000 * 13);
    }

    #[test]
    fn resamples_48k_to_16k() {
        let mut segmenter = SpeechSegmenter::default();
        let output = segmenter.push(&vec![0.1; 48_000], 48_000);
        assert!(output.is_empty());
        assert!(segmenter.speech.len() > 15_000 && segmenter.speech.len() < 17_000);
    }
}
