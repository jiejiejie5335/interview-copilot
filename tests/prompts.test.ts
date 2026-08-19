import { describe, expect, it } from "vitest";
import { buildInterviewMessages } from "../src/lib/prompts";

describe("buildInterviewMessages", () => {
  it("forces the explicitly selected response language", () => {
    const messages = buildInterviewMessages("请介绍 GMP", "en", []);
    expect(messages[0].content).toContain("explicitly selected English");
    expect(messages[0].content).toContain("MUST answer entirely");
  });

  it("includes fragmented talking-point hints without treating them as facts", () => {
    const messages = buildInterviewMessages(
      "如何优化 Go 调度性能？",
      "zh",
      [],
      "",
      "",
      "GMP；减少上下文切换；结合支付项目",
    );
    expect(messages[0].content).toContain("GMP；减少上下文切换；结合支付项目");
    expect(messages[0].content).toContain("Do not treat them as verified experience");
  });
});
