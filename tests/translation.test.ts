import { describe, expect, it } from "vitest";
import { normalizeOllamaBaseUrl } from "../src/lib/translation";

describe("normalizeOllamaBaseUrl", () => {
  it("removes trailing slashes", () => {
    expect(normalizeOllamaBaseUrl("http://127.0.0.1:11434///")).toBe(
      "http://127.0.0.1:11434",
    );
  });

  it("rejects an empty address", () => {
    expect(() => normalizeOllamaBaseUrl("   ")).toThrow("Ollama 地址不能为空");
  });
});
