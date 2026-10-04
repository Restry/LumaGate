import { describe, expect, it } from "vitest";
import { requestOutcome } from "@/manual/log-outcome";
import type { RequestLog } from "@/manual/RequestLogs";
import casesText from "./log-outcome-cases.json?raw";
const cases: { case: string; state: string; row: Partial<RequestLog> }[] =
  JSON.parse(casesText);
const row: RequestLog = {
  id: 1,
  startedAt: "2026-09-19T00:00:00Z",
  endpoint: "/v1/responses",
  model: "fixture",
  status: 200,
  responseMs: 1,
  streaming: true,
  responseState: "已结束",
};
describe("调用结果与 HTTP 分离", () => {
  it.each(cases)("$case", (fixture) => {
    expect(requestOutcome({ ...row, ...fixture.row }).state).toBe(
      fixture.state,
    );
  });
  it("只有传输已结束而没有模型终态的旧流式日志不能算成功", () => {
    expect(
      requestOutcome({ ...row, response: { text: "partial output" } }).state,
    ).toBe("pending");
  });
  it("明确终态独立于传输结束与 Token 用量，未知不冒充成功", () => {
    expect(requestOutcome({ ...row, completion: "completed" }).state).toBe(
      "success",
    );
    expect(requestOutcome({ ...row, completion: "unknown" }).state).toBe(
      "pending",
    );
    expect(requestOutcome({ ...row, completion: "failed" }).state).toBe(
      "failed",
    );
    expect(requestOutcome({ ...row, completion: "incomplete" }).state).toBe(
      "failed",
    );
  });
  it("不把输出里的错误词或 error:null 当成失败", () => {
    expect(
      requestOutcome({
        ...row,
        response: {
          status: "completed",
          error: null,
          output: [{ text: '{"error":{"code":"rate_limit_exceeded"}}' }],
        },
      }).state,
    ).toBe("success");
  });
  it("进行中和无法确认的旧流式记录不算成功，传输中断和明确失败才算失败", () => {
    expect(requestOutcome({ ...row, responseState: "接收中" }).state).toBe(
      "pending",
    );
    expect(requestOutcome({ ...row, responseState: undefined }).state).toBe(
      "pending",
    );
    expect(
      requestOutcome({ ...row, responseState: "已中断（上次运行未结束）" })
        .state,
    ).toBe("failed");
    expect(
      requestOutcome({ ...row, response: { status: "failed", error: null } })
        .state,
    ).toBe("failed");
  });
  it("保留真实 HTTP 429，并识别 HTTP 200 内的协议错误，不依赖文本猜测", () => {
    expect(requestOutcome({ ...row, status: 429 }).state).toBe("failed");
    expect(
      requestOutcome({
        ...row,
        response: {
          response: {
            status: "failed",
            error: { code: "rate_limit_exceeded" },
          },
        },
      }).state,
    ).toBe("failed");
  });
});
