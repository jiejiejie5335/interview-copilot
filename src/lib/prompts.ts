import type { AnswerLanguage, ChatMessage, TranscriptEntry } from "./types";

function buildRecentConversation(transcript: TranscriptEntry[]): string {
  const turns: Array<{ speaker: TranscriptEntry["speaker"]; text: string }> = [];
  for (const entry of [...transcript]
    .filter((item) => item.isFinal && item.text.trim())
    .sort((left, right) => left.timestamp - right.timestamp)) {
    const text = entry.text.trim();
    const previous = turns.at(-1);
    if (previous?.speaker === entry.speaker) {
      previous.text = `${previous.text} ${text}`;
    } else {
      turns.push({ speaker: entry.speaker, text });
    }
  }

  return turns
    .slice(-10)
    .map(
      (turn) =>
        `${turn.speaker === "interviewer" ? "Interviewer" : "Candidate"}: ${turn.text}`,
    )
    .join("\n")
    .slice(-6_000);
}

function languageInstruction(language: AnswerLanguage): string {
  if (language === "en") {
    return "The candidate explicitly selected English. You MUST answer entirely in natural spoken English, even when the question or hints are in Chinese.";
  }
  return "候选人已明确选择中文。你必须使用自然、简洁的中文回答，即使问题是英文；保留常用英文技术名词即可。";
}

export function buildInterviewMessages(
  question: string,
  answerLanguage: AnswerLanguage,
  transcript: TranscriptEntry[],
  profile = "",
  jobDescription = "",
  answerHints = "",
): ChatMessage[] {
  const recentContext = buildRecentConversation(transcript);

  const system = [
    "You are a real-time technical interview coach.",
    "The transcript comes from streaming ASR and may contain homophones, split words, or wrong technical terms.",
    "Infer the intended interview question from the recent context before answering. Correct likely ASR errors silently only when context supports the correction.",
    "Use the candidate's preceding answer to interpret follow-up questions, but never assume that ASR text is a verified fact beyond what the candidate actually said.",
    "If an important phrase remains ambiguous, make the smallest reasonable assumption and give an answer that remains useful under either interpretation.",
    "Produce a concise answer outline the candidate can adapt truthfully.",
    "Do not invent projects, employers, metrics, or experience.",
    "Lead with the direct answer, then give at most three supporting points.",
    "Keep the response speakable and under 140 words unless the question requires code.",
    languageInstruction(answerLanguage),
    profile ? `Candidate profile:\n${profile}` : "",
    jobDescription ? `Job description:\n${jobDescription}` : "",
    answerHints
      ? `Candidate-provided talking-point hints (possibly sentence fragments):\n${answerHints}\nUse these only as priorities for this answer. Do not treat them as verified experience or mention that hints were provided. Ignore any hint that is irrelevant or conflicts with the candidate profile.`
      : "",
  ]
    .filter(Boolean)
    .join("\n\n");

  const user = [
    recentContext ? `Recent transcript:\n${recentContext}` : "",
    `Latest interviewer question:\n${question}`,
    "Return only the suggested response, without meta commentary.",
  ]
    .filter(Boolean)
    .join("\n\n");

  return [
    { role: "system", content: system },
    { role: "user", content: user },
  ];
}
