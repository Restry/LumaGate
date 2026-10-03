import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { RequestLogs, type RequestLog } from "@/manual/RequestLogs";
import type { HistoryQuery, HistoryReply } from "@/manual/log-history";
import { historyFixture, readyStorage } from "./history-fixture";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const rpc = vi.mocked(invoke);
const base: RequestLog = {
  id: 2,
  startedAt: "2026-09-20T00:00:00Z",
  endpoint: "/v1/responses",
  model: "model-a",
  status: 200,
  responseMs: 12,
  streaming: true,
  responseState: "已结束",
  completion: "completed",
  providers: [{ id: "a", name: "Alpha", outcome: "已接收响应" }],
};
let rows: RequestLog[];
beforeEach(() => {
  window.localStorage.clear();
  rpc.mockReset();
  rows = [
    base,
    {
      ...base,
      id: 1,
      status: 403,
      completion: "failed",
      streaming: false,
      model: null,
      providers: [],
    },
  ];
  rpc.mockImplementation(async (command, args) =>
    command === "manual_query_logs"
      ? historyFixture(rows, (args as { query: HistoryQuery }).query)
      : undefined,
  );
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});
const table = () => screen.findByRole("table", { name: "网关调用日志" });
async function more(user: ReturnType<typeof userEvent.setup>) {
  if (!screen.queryByRole("heading", { name: "更多筛选" }))
    await user.click(screen.getByRole("button", { name: "更多筛选" }));
}
const statusFilter = () =>
  screen.getByRole("combobox", { name: "按调用结果或 HTTP 状态筛选" });

describe("历史日志查询与恢复", () => {
  it("刷新等待期间保留旧数值与节点，不插入跳动的加载段落", async () => {
    const user = userEvent.setup();
    render(<RequestLogs />);
    await table();
    const total = within(
      screen.getByLabelText("当前筛选全部记录统计"),
    ).getByText("总数").nextElementSibling;
    let finish!: (value: HistoryReply) => void;
    rpc.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    await user.click(screen.getByRole("button", { name: "刷新" }));
    expect(total).toHaveTextContent("2");
    expect(
      within(screen.getByLabelText("当前筛选全部记录统计")).getByText("总数")
        .nextElementSibling,
    ).toBe(total);
    expect(screen.queryByText(/正在查询/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "刷新" })).not.toBeDisabled();
    expect(screen.getByRole("button", { name: "刷新" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
    await act(async () => {
      finish(historyFixture([...rows, { ...base, id: 3 }]));
    });
    expect(total).toHaveTextContent("3");
  });
  it("简短标签、互斥视图与渐进筛选不删除原有条件", async () => {
    const user = userEvent.setup();
    render(<RequestLogs />);
    await table();
    expect(screen.getByLabelText("当前筛选全部记录统计")).toHaveTextContent(
      "总数",
    );
    expect(screen.queryByText("总调用数")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("combobox", { name: "按接口筛选" }),
    ).not.toBeInTheDocument();
    await more(user);
    expect(screen.getByRole("combobox", { name: "按接口筛选" })).toBeVisible();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("tab", { name: "分析" }));
    await screen.findByLabelText("Token 用量分析");
    expect(
      screen.queryByRole("table", { name: "网关调用日志" }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "日志" }));
    await table();
  });
  it("默认列表优先，分析展开偏好保留，精确传输和 HTTP 字段不丢", async () => {
    const user = userEvent.setup();
    let view = render(<RequestLogs />);
    await table();
    expect(screen.queryByLabelText("Token 用量分析")).not.toBeInTheDocument();
    expect(screen.getAllByText("12 ms")[0]).toBeVisible();
    expect(screen.getByText(/HTTP 200 · 流式/)).toBeVisible();
    await user.click(screen.getByRole("tab", { name: "分析" }));
    await screen.findByLabelText("Token 用量分析");
    view.unmount();
    view = render(<RequestLogs />);
    await screen.findByLabelText("Token 用量分析");
    await user.click(screen.getByRole("tab", { name: "日志" }));
    view.unmount();
    render(<RequestLogs />);
    expect(screen.queryByLabelText("Token 用量分析")).not.toBeInTheDocument();
  });
  it("分页显示少量记录，但统计和 Token 覆盖全部匹配记录，不止 200 条", async () => {
    rows = Array.from({ length: 251 }, (_, i) => ({
      ...base,
      id: i,
      usage: {
        state: "reported",
        inputTokens: 100,
        outputTokens: 20,
        totalTokens: 120,
        cacheReadTokens: 80,
        cacheWriteTokens: null,
        reasoningTokens: null,
        reason: null,
      },
    }));
    const user = userEvent.setup();
    render(<RequestLogs />);
    await table();
    expect(screen.getByRole("status")).toHaveTextContent("1–50 / 251 条");
    expect(
      within(screen.getByLabelText("当前筛选全部记录统计")).getAllByText("251"),
    ).toHaveLength(2);
    await user.click(screen.getByRole("button", { name: "下一页" }));
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("51–100 / 251 条"),
    );
    expect(rpc.mock.calls.at(-1)?.[1]).toMatchObject({
      query: { page: 1, anchor: 250 },
    });
    await user.click(screen.getByRole("tab", { name: "分析" }));
    await screen.findByLabelText("Token 用量分析");
    expect(document.querySelector('[data-number-text="30.12K"]')).toBeVisible();
    await user.click(screen.getByRole("tab", { name: "日志" }));
    await user.click(screen.getByRole("button", { name: "刷新" }));
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("1–50 / 251 条"),
    );
  });
  it("模型、Provider（含回退）和错误码交集筛选，统计随范围改变", async () => {
    rows = [
      {
        ...base,
        id: 3,
        model: "FW-Kimi-K3",
        providers: [
          { id: "a", name: "Alpha", outcome: "429" },
          { id: "b", name: "Beta", outcome: "ok" },
        ],
        response: { id: "resp-kimi" },
      },
      {
        ...base,
        id: 2,
        model: "gpt-6",
        response: { status: "failed", error: { code: "rate_limit_exceeded" } },
      },
      {
        ...base,
        id: 1,
        model: "FW-Kimi-K3",
        providers: [{ id: "b", name: "Beta", outcome: "ok" }],
      },
    ];
    const user = userEvent.setup();
    render(<RequestLogs />);
    await table();
    await user.type(screen.getByRole("textbox", { name: "模型" }), "KIMI");
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("/ 2 条"),
    );
    await user.click(
      screen.getByRole("combobox", { name: "按 Provider 筛选（含回退尝试）" }),
    );
    await user.click(screen.getByRole("option", { name: "Alpha (a)" }));
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("/ 1 条"),
    );
    await more(user);
    await user.type(
      screen.getByRole("textbox", { name: "请求 ID / 错误码" }),
      "resp-kimi",
    );
    await waitFor(() =>
      expect(rpc.mock.calls.at(-1)?.[1]).toMatchObject({
        query: { search: "resp-kimi" },
      }),
    );
    await user.click(screen.getByRole("button", { name: "清除筛选" }));
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("/ 3 条"),
    );
    await more(user);
    await user.type(
      screen.getByRole("textbox", { name: "请求 ID / 错误码" }),
      "rate_limit_exceeded",
    );
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("/ 1 条"),
    );
    expect(screen.getByLabelText("当前筛选全部记录统计")).toHaveTextContent(
      "0.0%",
    );
  });
  it("HTTP 200 内限流仍在业务失败中，具体 HTTP 筛选保留原始码", async () => {
    rows = [
      {
        ...base,
        response: { status: "failed", error: { code: "rate_limit_exceeded" } },
      },
    ];
    const user = userEvent.setup();
    render(<RequestLogs />);
    await table();
    expect(screen.getByText("限流（429）")).toBeVisible();
    expect(screen.getByText(/HTTP 200/)).toBeVisible();
    await more(user);
    await user.click(statusFilter());
    await user.click(screen.getByRole("option", { name: "失败" }));
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("/ 1 条"),
    );
    await user.click(screen.getByRole("button", { name: "展开请求 2 的详情" }));
    expect(
      screen.getByRole("region", { name: "请求 2 的详情" }),
    ).toHaveTextContent("Response · 调用失败 · 限流（429）");
  });
  it("密钥与来源按稳定 ID 显示，已删除历史身份仍可选择", async () => {
    rows = [
      { ...base, caller: { kind: "key", keyId: "old-key", name: "Old name" } },
      { ...base, id: 1, caller: { kind: "local" } },
    ];
    const user = userEvent.setup();
    render(<RequestLogs />);
    await table();
    await more(user);
    await user.click(screen.getByRole("combobox", { name: "按访问密钥筛选" }));
    await user.click(
      screen.getByRole("option", { name: "Old name (old-key)" }),
    );
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("/ 1 条"),
    );
    await user.click(screen.getByRole("button", { name: "展开请求 2 的详情" }));
    expect(
      screen.getByRole("region", { name: "请求 2 的详情" }),
    ).toHaveTextContent("old-key");
  });
  it("空结果和未加载不伪造成功率", async () => {
    rows = [];
    render(<RequestLogs />);
    expect(
      within(screen.getByLabelText("当前筛选全部记录统计")).getAllByText("—"),
    ).toHaveLength(6);
    await screen.findByRole("heading", { name: "没有匹配记录" });
    expect(
      within(screen.getByLabelText("当前筛选全部记录统计")).getAllByText("0"),
    ).toHaveLength(4);
    expect(
      within(screen.getByLabelText("当前筛选全部记录统计")).getAllByText("—"),
    ).toHaveLength(2); // 无记录时成功率与未报告的 Token 都未知。
  });
  it("读取失败不回显异常内容，手动刷新能恢复", async () => {
    rpc.mockRejectedValueOnce(new Error("PRIVATE_ERROR_CONTENT"));
    const user = userEvent.setup();
    render(<RequestLogs />);
    expect(await screen.findByRole("alert")).toHaveTextContent("更新失败");
    expect(screen.queryByText(/PRIVATE_ERROR_CONTENT/)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "刷新" }));
    await table();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
  it("库不可用显示恢复页，不清空历史，不自动启动网关", async () => {
    let ready = false;
    rpc.mockImplementation(async (command, args) => {
      if (command === "manual_query_logs")
        return ready
          ? historyFixture(rows, (args as { query: HistoryQuery }).query)
          : {
              storage: {
                ...readyStorage,
                ready: false,
                message: "日志库不可用，历史文件未被删除",
              },
            };
      if (command === "manual_retry_log_storage") {
        ready = true;
        return readyStorage;
      }
    });
    const user = userEvent.setup(),
      changed = vi.fn();
    render(<RequestLogs onStorageChange={changed} />);
    await screen.findByRole("heading", { name: "日志存储需要处理" });
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "打开日志目录" }));
    expect(rpc).toHaveBeenCalledWith("manual_open_log_directory");
    await user.click(screen.getByRole("button", { name: "重试打开日志库" }));
    await table();
    expect(changed).toHaveBeenCalled();
    expect(rpc.mock.calls.some(([cmd]) => cmd === "manual_gateway")).toBe(
      false,
    );
  });
  it("运行中的网关不会被恢复按钮偷偷停止", async () => {
    rpc.mockResolvedValue({
      storage: { ...readyStorage, ready: false, message: "日志存储不可用" },
    });
    render(<RequestLogs gatewayRunning />);
    expect(
      await screen.findByRole("button", { name: "重试打开日志库" }),
    ).toBeDisabled();
    expect(screen.getByText(/请先在顶部手动停止网关/)).toBeVisible();
  });
  it("行详情支持键盘、选中文字不触发折叠，刷新保持同一请求展开", async () => {
    const user = userEvent.setup();
    render(<RequestLogs />);
    await table();
    const toggle = screen.getByRole("button", { name: "展开请求 2 的详情" });
    toggle.focus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("region", { name: "请求 2 的详情" })).toBeVisible();
    const range = document.createRange();
    range.selectNodeContents(screen.getByText("model-a").firstChild!);
    window.getSelection()?.removeAllRanges();
    window.getSelection()?.addRange(range);
    expect(window.getSelection()?.toString()).toBe("model-a");
    fireEvent.click(screen.getByText("model-a").closest("tr")!);
    expect(screen.getByRole("region", { name: "请求 2 的详情" })).toBeVisible();
    window.getSelection()?.removeAllRanges();
    rows = [{ ...base, response: { text: "updated" } }];
    await user.click(screen.getByRole("button", { name: "刷新" }));
    await waitFor(() =>
      expect(
        screen.getByRole("region", { name: "请求 2 的详情" }),
      ).toHaveTextContent('"text": "updated"'),
    );
  });
  it("今天的第一页跨午夜刷新日期，但翻页固定原查询日期", async () => {
    vi.setSystemTime(new Date(2026, 8, 20, 23, 59));
    rows = Array.from({ length: 151 }, (_, i) => ({
      ...base,
      id: i,
      startedAt: "2026-09-20T12:00:00Z",
    }));
    const user = userEvent.setup();
    render(<RequestLogs />);
    await table();
    await user.click(screen.getByRole("combobox", { name: "日志时间范围" }));
    await user.click(screen.getByRole("option", { name: "今天" }));
    await waitFor(() =>
      expect(rpc.mock.calls.at(-1)?.[1]).toMatchObject({
        query: { from: new Date(2026, 8, 20).getTime() },
      }),
    );
    await user.click(screen.getByRole("button", { name: "下一页" }));
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("51–100"),
    );
    vi.setSystemTime(new Date(2026, 8, 21, 0, 1));
    await user.click(screen.getByRole("button", { name: "下一页" }));
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("101–150"),
    );
    expect(rpc.mock.calls.at(-1)?.[1]).toMatchObject({
      query: { from: new Date(2026, 8, 20).getTime() },
    });
    await user.click(screen.getByRole("button", { name: "刷新" }));
    await waitFor(() =>
      expect(rpc.mock.calls.at(-1)?.[1]).toMatchObject({
        query: { from: new Date(2026, 8, 21).getTime() },
      }),
    );
  });
  it("今天第一页自动刷新会跨日，不一直停留在昨天", async () => {
    vi.setSystemTime(new Date(2026, 8, 20, 23, 59));
    const user = userEvent.setup();
    render(<RequestLogs />);
    await table();
    await user.click(screen.getByRole("combobox", { name: "日志时间范围" }));
    await user.click(screen.getByRole("option", { name: "今天" }));
    await waitFor(() =>
      expect(rpc.mock.calls.at(-1)?.[1]).toMatchObject({
        query: { from: new Date(2026, 8, 20).getTime() },
      }),
    );
    vi.setSystemTime(new Date(2026, 8, 21, 0, 1));
    await waitFor(
      () =>
        expect(rpc.mock.calls.at(-1)?.[1]).toMatchObject({
          query: { from: new Date(2026, 8, 21).getTime() },
        }),
      { timeout: 4500 },
    );
  });
  it("初次请求未结束不叠加轮询，离开工作区不再读取", async () => {
    let finish!: (v: HistoryReply) => void;
    rpc.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const view = render(<RequestLogs />);
    expect(rpc).toHaveBeenCalledTimes(1);
    await act(async () => {
      finish(historyFixture(rows));
    });
    await table();
    view.unmount();
    expect(rpc).toHaveBeenCalledTimes(1);
  });
});
