import type { DetectedLanguage, QuestionDetection } from "./types";

const EN_STARTERS = [
  /^(?:so[,.]?\s+|okay[,.]?\s+|and\s+)?(?:what|why|how|when|where|who|which)\b/i,
  /^(?:can|could|would|will|should|do|does|did|is|are|was|were|have|has)\s+(?:you|we|i|it|there)\b/i,
  /^(?:tell me about|walk me through|describe|explain|give me an example|share an example)\b/i,
];

const EN_INTERVIEW_PATTERNS = [
  /\bwhat(?:'s| is) your experience\b/i,
  /\bhow (?:do|did|would|have) you\b/i,
  /\bdescribe a time\b/i,
  /\btell me about a time\b/i,
  /\bwhat would you do\b/i,
  /\bcan you (?:explain|describe|tell|walk)\b/i,
];

const ZH_PATTERNS = [
  /(?:什麼|什么|為何|为何|為什麼|为什么|怎麼|怎么|如何|哪(?:個|个|些|裡|里)|多少|是否|有沒有|有没有|能否|可否|是不是|對嗎|对吗|嗎|吗|呢)[？?]?$/u,
  /^(?:請|请|能否|可以|能不能|可不可以)?(?:介紹|介绍|說明|说明|解釋|解释|描述|談談|谈谈|講講|讲讲|舉例|举例|分享|分析)/u,
  /(?:你會怎麼|你会怎么|你是怎麼|你是怎么|你如何|你為什麼|你为什么|你認為|你认为|你的經驗|你的经验|遇到過|遇到过|負責過|负责过|做過哪些|做过哪些|如何處理|如何处理|如何設計|如何设计)/u,
  /(?:請問|请问|想問一下|想问一下|我想了解|方便說說|方便说说)/u,
];

const NON_QUESTION = [
  /^(?:好|好的|明白|可以|谢谢|嗯|对|没问题)[。.!！]?$/u,
  /^(?:okay|ok|great|good|thanks|thank you|got it|makes sense)[.!]?$/i,
];

function languageOf(text: string): DetectedLanguage {
  const han = (text.match(/[\u3400-\u9fff]/gu) ?? []).length;
  const latin = (text.match(/[A-Za-z]/g) ?? []).length;
  if (han > 0 && latin > 0) return "mixed";
  if (han > 0) return "zh";
  if (latin > 0) return "en";
  return "unknown";
}

export function detectQuestion(input: string): QuestionDetection {
  const text = input.replace(/\s+/g, " ").trim();
  const language = languageOf(text);

  if (!text) {
    return {
      isQuestion: false,
      confidence: 0,
      language,
      normalizedText: text,
      reason: "empty",
    };
  }

  if (NON_QUESTION.some((pattern) => pattern.test(text))) {
    return {
      isQuestion: false,
      confidence: 0.94,
      language,
      normalizedText: text,
      reason: "acknowledgement",
    };
  }

  let confidence = /[?？]\s*$/.test(text) ? 0.93 : 0;
  let reason = confidence > 0 ? "question-mark" : "no-pattern";

  if (ZH_PATTERNS.some((pattern) => pattern.test(text))) {
    confidence = Math.max(confidence, 0.86);
    reason = "zh-interview-pattern";
  }

  if (EN_STARTERS.some((pattern) => pattern.test(text))) {
    confidence = Math.max(confidence, 0.84);
    reason = "en-question-starter";
  }

  if (EN_INTERVIEW_PATTERNS.some((pattern) => pattern.test(text))) {
    confidence = Math.max(confidence, 0.89);
    reason = "en-interview-pattern";
  }

  // ASR often omits punctuation. Longer utterances containing both Chinese
  // and English technical terms still count when a Chinese interrogative
  // pattern is present.
  if (language === "mixed" && /(?:怎麼|怎么|如何|為什麼|为什么|介紹|介绍|解釋|解释|what|how|why)/iu.test(text)) {
    confidence = Math.max(confidence, 0.82);
    reason = "mixed-language-pattern";
  }

  return {
    isQuestion: confidence >= 0.72,
    confidence,
    language,
    normalizedText: text,
    reason,
  };
}

export function preferredAnswerLanguage(language: DetectedLanguage): "zh" | "en" {
  return language === "en" ? "en" : "zh";
}
