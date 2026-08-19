import { describe, expect, it } from "vitest";
import { normalizeOpenAIBaseUrl } from "../src/lib/provider";

describe("normalizeOpenAIBaseUrl", () => {
  it("adds /v1 for DeepSeek", () => {
    expect(normalizeOpenAIBaseUrl("https://api.deepseek.com")).toBe(
      "https://api.deepseek.com/v1",
    );
  });

  it("does not duplicate /v1", () => {
    expect(normalizeOpenAIBaseUrl("https://example.com/v1/")).toBe(
      "https://example.com/v1",
    );
  });
});
