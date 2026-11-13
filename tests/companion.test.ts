import { describe, expect, it } from "vitest";
import { buildLiveInterviewerTranscript } from "../src/lib/companion";
import type { TranscriptEntry } from "../src/lib/types";

function entry(
  id: string,
  speaker: TranscriptEntry["speaker"],
  text: string,
): TranscriptEntry {
  return {
    id,
    speaker,
    text,
    language: "zh",
    timestamp: Number(id),
    isFinal: true,
  };
}

describe("buildLiveInterviewerTranscript", () => {
  it("shows only the current interviewer turn plus its partial text", () => {
    const transcript = [
      entry("1", "interviewer", "请先介绍一下自己"),
      entry("2", "candidate", "这是我的自我介绍"),
      entry("3", "interviewer", "项目里为什么使用 Redis？"),
      entry("4", "interviewer", "请说明缓存一致性"),
    ];

    expect(
      buildLiveInterviewerTranscript(
        transcript,
        entry("5", "interviewer", "以及缓存击穿怎么处理"),
      ),
    ).toBe("项目里为什么使用 Redis？\n请说明缓存一致性\n以及缓存击穿怎么处理");
  });

  it("ignores a candidate partial transcript", () => {
    expect(
      buildLiveInterviewerTranscript(
        [entry("1", "interviewer", "你负责哪些模块？")],
        entry("2", "candidate", "我主要负责"),
      ),
    ).toBe("你负责哪些模块？");
  });
});
