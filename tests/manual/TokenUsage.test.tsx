import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { RequestLogs, type RequestLog } from "@/manual/RequestLogs";
import { historyFixture } from "./history-fixture";
import type { HistoryQuery } from "@/manual/log-history";
import {
  completeUsage,
  cacheUsageAnalytics,
  formatTokens,
  compactTokens,
  usageAnalytics,
  usageEligible,
  usageLabel,
  type TokenUsage,
} from "@/manual/log-usage";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@/manual/TokenCharts", () => ({
  default: ({ analytics }: { analytics: { total: number } }) => (
    <div>图表合计：{analytics.total}</div>
  ),
}));
const rpc = vi.mocked(invoke);
function respondWith(values: RequestLog[]) {
  rpc.mockImplementation(async (_command, args) =>
    historyFixture(values, (args as { query: HistoryQuery }).query),
  );
}
function usage(input = 100, output = 20): TokenUsage {
  return {
    state: "reported",
    inputTokens: input,
    outputTokens: output,
    totalTokens: input + output,
    cacheReadTokens: Math.min(40, input),
    cacheWriteTokens: null,
    reasoningTokens: Math.min(5, output),
    reason: null,
  };
}
function row(id: number, values: Partial<RequestLog> = {}): RequestLog {
  return {
    id,
    startedAt: `2026-09-17T10:0${id}:00Z`,
    endpoint: "/v1/responses",
    model: "gpt-fixture",
    status: 200,
    responseMs: 100,
    streaming: false,
    providers: [{ id: "p", name: "P", outcome: "ok" }],
    responseState: "已结束",
    usage: usage(),
    ...values,
  };
}
beforeEach(() => {
  window.localStorage.clear();
  window.localStorage.setItem("cc-switch-manual.logs.analysis", "true");
  rpc.mockReset();
});

describe("Token 用量口径", () => {
  it("K/M 只缩写显示，完整值仍保持整数精度", () => {
    expect(compactTokens(25040781)).toBe("25.04M");
    expect(compactTokens(316070)).toBe("316.07K");
    expect(compactTokens(306)).toBe("306");
    expect(compactTokens(0)).toBe("0");
    expect(formatTokens(25040781)).toBe("25,040,781");
    expect(formatTokens(null)).toBe("—");
  });
  it("缓存汇总只使用已报告字段的同一范围，不把未知混进分母", () => {
    const values = [
      row(1, { usage: { ...usage(316070, 240), cacheReadTokens: 315764 } }),
      row(2, { usage: { ...usage(10000, 10), cacheReadTokens: null } }),
    ];
    const cache = cacheUsageAnalytics(values);
    expect(cache).toMatchObject({
      complete: 2,
      reported: 1,
      read: 315764,
      nonRead: 306,
    });
    expect(cache.share).toBeCloseTo(315764 / 316070);
    expect(
      cacheUsageAnalytics([
        row(1, { usage: { ...usage(), cacheReadTokens: 0 } }),
      ]),
    ).toMatchObject({ reported: 1, read: 0, nonRead: 100, share: 0 });
    expect(
      cacheUsageAnalytics([
        row(1, { usage: { ...usage(), cacheReadTokens: null } }),
      ]),
    ).toMatchObject({ reported: 0, read: null, nonRead: null, share: null });
    expect(
      cacheUsageAnalytics([
        row(1, { usage: { ...usage(), cacheReadTokens: 101 } }),
      ]).reported,
    ).toBe(0);
    expect(
      cacheUsageAnalytics([row(1, { usage: usage(0, 0) })]).share,
    ).toBeNull();
  });
  it("首页显示缓存读取、非缓存读取输入、占比与覆盖数", async () => {
    respondWith([
      row(1, { usage: { ...usage(316070, 240), cacheReadTokens: 315764 } }),
    ]);
    render(<RequestLogs />);
    const panel = await screen.findByLabelText("当前筛选的缓存 Token 统计");
    await waitFor(() =>
      expect(panel.querySelector('[data-number-text="315.76K"]')).toBeVisible(),
    );
    expect(within(panel).getByText("306")).toBeVisible();
    expect(panel.querySelector('[data-number-text="99.90%"]')).toBeVisible();
    expect(panel.closest(".ls-usage-card")).toHaveTextContent("1 / 1");
  });
  it("只汇总完整用量，不重复加入缓存/推理，不从预览推算旧记录", () => {
    const rows = [
      row(1),
      row(2, { usage: usage(50, 10) }),
      row(3, {
        usage: null,
        response: { usage: { input_tokens: 9999, output_tokens: 9999 } },
      }),
      row(4, {
        usage: {
          ...usage(),
          state: "partial",
          outputTokens: null,
          totalTokens: null,
        },
      }),
    ];
    const stats = usageAnalytics(rows, "model");
    expect(stats.input).toBe(150);
    expect(stats.output).toBe(30);
    expect(stats.total).toBe(180);
    expect(stats.reported).toBe(2);
    expect(stats.partial).toBe(1);
    expect(stats.unavailable).toBe(1);
  });
  it("真实零值可统计，缺失不是零；目录和未转发请求不计入覆盖率", () => {
    expect(usageAnalytics([row(1, { usage: null })], "model").total).toBeNull();
    expect(
      usageAnalytics([row(1, { usage: usage(0, 0) })], "model").total,
    ).toBe(0);
    expect(usageEligible(row(1, { endpoint: "/v1/models" }))).toBe(false);
    expect(usageEligible(row(1, { providers: [], status: 401 }))).toBe(false);
    expect(
      usageAnalytics(
        [row(1, { endpoint: "/v1/models" }), row(2, { providers: [] })],
        "key",
      ).eligible,
    ).toBe(0);
    expect(formatTokens(null)).toBe("—");
    expect(formatTokens(0)).toBe("0");
  });
  it("进行中、部分用量和无有效用量分别计数，异常数字不进入图表", () => {
    const rows = [
      row(1, { usage: null, responseState: "接收中" }),
      row(2, { usage: { ...usage(), state: "partial" } }),
      row(3, { usage: { ...usage(), inputTokens: -1 } }),
      row(4, { usage: { ...usage(), totalTokens: 999 } }),
    ];
    const stats = usageAnalytics(rows, "model");
    expect(stats.pending).toBe(1);
    expect(stats.partial).toBe(1);
    expect(stats.unavailable).toBe(2);
    expect(stats.total).toBeNull();
    expect(completeUsage(rows[2])).toBe(false);
    expect(usageLabel(rows[3])).toBe("用量格式异常");
  });
  it("按稳定密钥 ID 归组并使用最近调用名称，不混并同名不同密钥", () => {
    const rows = [
      row(3, { caller: { kind: "key", keyId: "a", name: "新名称" } }),
      row(1, { caller: { kind: "key", keyId: "a", name: "旧名称" } }),
      row(2, { caller: { kind: "key", keyId: "b", name: "新名称" } }),
      row(4, { caller: { kind: "local" } }),
    ];
    const stats = usageAnalytics(rows, "key");
    expect(stats.ranking).toHaveLength(3);
    expect(stats.ranking[0].label).toBe("新名称 (a)");
    expect(stats.ranking[0].requests).toBe(2);
    expect(stats.total).toBe(480);
  });
  it("Top5 的其他分组不丢用量；时间轴保留空时间桶而不伪造请求", () => {
    const rows = Array.from({ length: 8 }, (_, i) =>
      row(i, { model: `model-${i}`, usage: usage(i + 1, 1) }),
    );
    const stats = usageAnalytics(rows, "model");
    expect(stats.distribution).toHaveLength(6);
    expect(stats.distribution.at(-1)?.label).toBe("其他（3项）");
    expect(stats.distribution.reduce((s, p) => s + p.total, 0)).toBe(
      stats.total,
    );
    const time = usageAnalytics(
      [
        row(1, { startedAt: "2026-09-17T10:00:01Z" }),
        row(2, { startedAt: "2026-09-17T10:02:01Z" }),
      ],
      "model",
    );
    expect(time.timeline).toHaveLength(3);
    expect(time.timeline[1].requests).toBe(0);
    expect(time.reported).toBe(2);
  });
  it("无效时间不影响模型总量，也不触发图表日期异常", () => {
    const stats = usageAnalytics(
      [row(1, { startedAt: "invalid" }), row(2)],
      "model",
    );
    expect(stats.total).toBe(240);
    expect(stats.invalidTimes).toBe(1);
    expect(stats.timeline).toHaveLength(1);
  });
});

describe("Token 日志界面", () => {
  it("用量、图表和调用汇总跟随同一筛选；可收起图表", async () => {
    respondWith([
      row(1, { caller: { kind: "key", keyId: "a", name: "A" } }),
      row(2, {
        endpoint: "/v1/chat/completions",
        usage: usage(300, 30),
        caller: { kind: "key", keyId: "b", name: "B" },
      }),
    ]);
    const user = userEvent.setup();
    render(<RequestLogs />);
    const stats = await screen.findByLabelText("当前筛选的 Token 统计");
    expect(
      within(stats).getByText("总用量").nextElementSibling,
    ).toHaveTextContent("450");
    expect(await screen.findByText("图表合计：450")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "更多筛选" }));
    await user.click(screen.getByRole("combobox", { name: "按接口筛选" }));
    await user.click(screen.getByRole("option", { name: "Responses" }));
    await waitFor(() =>
      expect(
        within(stats).getByText("总用量").nextElementSibling,
      ).toHaveTextContent("120"),
    );
    expect(screen.getByText("图表合计：120")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "收起图表" }));
    expect(screen.queryByText("图表合计：120")).not.toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "日志" }));
    expect(
      within(screen.getByLabelText("当前筛选全部记录统计")).getByText("总数")
        .nextElementSibling,
    ).toHaveTextContent("1");
    await user.click(screen.getByRole("button", { name: "展开请求 1 的详情" }));
    expect(
      screen.getByRole("region", { name: "请求 1 的详情" }),
    ).toHaveTextContent("缓存读取");
    expect(
      rpc.mock.calls.every(([command]) => command === "manual_query_logs"),
    ).toBe(true);
  });
  it("没有 usage 时提示未知，不画示例曲线或显示零消耗", async () => {
    respondWith([row(1, { usage: null })]);
    render(<RequestLogs />);
    const stats = await screen.findByLabelText("当前筛选的 Token 统计");
    expect(
      within(stats).getByText("总用量").nextElementSibling,
    ).toHaveTextContent("—");
    expect(screen.getByText(/暂无完整用量/)).toBeVisible();
    expect(screen.queryByText(/图表合计/)).not.toBeInTheDocument();
  });
});
