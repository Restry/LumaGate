import { describe, expect, it } from "vitest";
import { historyDateBounds, localDateInput } from "@/manual/log-history";
describe("历史日期边界", () => {
  it("今天以本地午夜开始，结束为次日午夜，不误用 UTC 日期", () => {
    const now = new Date(2026, 8, 20, 18, 30);
    expect(historyDateBounds("today", "", "", now)).toEqual({
      from: new Date(2026, 8, 20).getTime(),
      to: new Date(2026, 8, 21).getTime(),
    });
    expect(historyDateBounds("week", "", "", now)).toEqual({
      from: new Date(2026, 8, 14).getTime(),
      to: new Date(2026, 8, 21).getTime(),
    });
  });
  it("自定义包含结束日期当天，拒绝倒置和无效日历日期", () => {
    expect(historyDateBounds("custom", "2026-09-19", "2026-09-20")).toEqual({
      from: new Date(2026, 8, 19).getTime(),
      to: new Date(2026, 8, 21).getTime(),
    });
    expect(() =>
      historyDateBounds("custom", "2026-02-30", "2026-03-01"),
    ).toThrow();
    expect(() =>
      historyDateBounds("custom", "2026-09-21", "2026-09-20"),
    ).toThrow();
    expect(() => historyDateBounds("custom", "", "2026-09-20")).toThrow();
  });
  it("全部时间不带日期上限，日期输入保留本机日历日期", () => {
    expect(historyDateBounds("all", "", "")).toEqual({});
    expect(localDateInput(new Date(2026, 0, 2, 1))).toBe("2026-01-02");
  });
});
