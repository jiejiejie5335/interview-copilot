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

  it("includes the candidate answer when interpreting an interviewer follow-up", () => {
    const messages = buildInterviewMessages("那缓存一致性怎么处理？", "zh", [
      {
        id: "question-1",
        speaker: "interviewer",
        text: "为什么在项目中使用 Redis？",
        language: "zh",
        timestamp: 1,
        isFinal: true,
      },
      {
        id: "answer-1",
        speaker: "candidate",
        text: "我主要用它降低数据库读取压力。",
        language: "zh",
        timestamp: 2,
        isFinal: true,
      },
    ]);

    expect(messages[1].content).toContain(
      "Candidate: 我主要用它降低数据库读取压力。",
    );
    expect(messages[1].content).toContain("Latest interviewer question:\n那缓存一致性怎么处理？");
  });
});
