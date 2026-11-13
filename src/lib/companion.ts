import type { TranscriptEntry } from "./types";

const MAX_LIVE_SEGMENTS = 6;
const MAX_LIVE_CHARACTERS = 3_000;

export function buildLiveInterviewerTranscript(
  transcript: TranscriptEntry[],
  partial?: TranscriptEntry,
): string {
  let lastCandidateIndex = -1;
  for (let index = transcript.length - 1; index >= 0; index -= 1) {
    if (transcript[index].speaker === "candidate") {
      lastCandidateIndex = index;
      break;
    }
  }
  const finalized = transcript
    .slice(lastCandidateIndex + 1)
    .filter((entry) => entry.speaker === "interviewer")
    .slice(-MAX_LIVE_SEGMENTS)
    .map((entry) => entry.text.trim())
    .filter(Boolean);
  const partialText = partial?.speaker === "interviewer" ? partial.text.trim() : "";
  const combined = [...finalized, partialText].filter(Boolean).join("\n");
  return combined.length <= MAX_LIVE_CHARACTERS
    ? combined
    : combined.slice(combined.length - MAX_LIVE_CHARACTERS);
}
