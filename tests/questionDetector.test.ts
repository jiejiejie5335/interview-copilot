import { describe, expect, it } from "vitest";
import { detectQuestion, preferredAnswerLanguage } from "../src/lib/questionDetector";

describe("detectQuestion", () => {
  it("detects a Chinese technical question without punctuation", () => {
    const result = detectQuestion("你会如何设计一个高并发的订单系统");
    expect(result.isQuestion).toBe(true);
    expect(result.language).toBe("zh");
  });

  it("detects an English behavioral prompt", () => {
    const result = detectQuestion("Tell me about a time you handled an outage");
    expect(result.isQuestion).toBe(true);
    expect(result.language).toBe("en");
  });

  it("detects mixed Chinese and English", () => {
    const result = detectQuestion("你会怎么优化 Kafka consumer lag");
    expect(result.isQuestion).toBe(true);
    expect(result.language).toBe("mixed");
  });

  it("detects Traditional Chinese emitted by multilingual STT", () => {
    const result = detectQuestion("請介紹一下你負責過的高併發系統，以及你是如何優化數據庫性能的。");
    expect(result.isQuestion).toBe(true);
    expect(result.language).toBe("zh");
  });

  it("rejects acknowledgements", () => {
    expect(detectQuestion("好的").isQuestion).toBe(false);
    expect(detectQuestion("makes sense").isQuestion).toBe(false);
  });

  it("follows English only for clearly English questions", () => {
    expect(preferredAnswerLanguage("en")).toBe("en");
    expect(preferredAnswerLanguage("mixed")).toBe("zh");
  });
});
