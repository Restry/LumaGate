import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import ManualApp from "@/manual/ManualApp";
import { RequestLogs, type RequestLog } from "@/manual/RequestLogs";
import type { Snapshot } from "@/manual/types";
import { historyFixture, readyStorage } from "./history-fixture";
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  isTauri: () => false,
}));
vi.mock("recharts", () => ({
  ResponsiveContainer: ({ children }: { children: ReactNode }) => (
    <div>{children}</div>
  ),
  AreaChart: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  Area: () => null,
  CartesianGrid: () => null,
  Tooltip: () => null,
  XAxis: () => null,
  YAxis: () => null,
}));
const rpc = vi.mocked(invoke);
let rows: RequestLog[];
let snapshot: Snapshot;
const calls = () => rpc.mock.calls.map(([command]) => command);
beforeEach(() => {
  localStorage.clear();
  document.documentElement.classList.remove("dark");
  snapshot = {
    document: {
      revision: 5,
      providers: [
        {
          id: "a",
          name: "Alpha",
          baseUrl: "https://a.example/v1",
          protocol: "openai_chat",
          enabled: true,
          models: [],
          keyEnv: "",
        },
      ],
      policies: {},
      tests: {},
      blockedModels: [],
    },
    groups: [],
    status: { running: false },
    baseUrl: "http://127.0.0.1:15722",
    logStorage: readyStorage,
  };
  const base: RequestLog = {
    id: 3,
    startedAt: new Date(Date.now() - 60000).toISOString(),
    endpoint: "/v1/responses",
    model: "real-model",
    status: 200,
    responseMs: 125,
    streaming: true,
    caller: { kind: "local" },
    providers: [{ id: "a", name: "Alpha", outcome: "已接收响应" }],
    completion: "completed",
    responseState: "已结束",
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
  };
  rows = [
    base,
    {
      ...base,
      id: 2,
      completion: "failed",
      usage: null,
      response: { status: "failed", error: { code: "rate_limit_exceeded" } },
    },
    { ...base, id: 1, completion: "unknown", usage: null },
  ];
  rpc.mockReset();
  rpc.mockImplementation(async (command, args) => {
    if (command === "manual_snapshot") return structuredClone(snapshot);
    if (command === "manual_query_logs")
      return historyFixture(
        rows,
        (args as { query: Parameters<typeof historyFixture>[1] }).query,
      );
    throw Error(`Unexpected mutation: ${command}`);
  });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});
describe("实际客户端 Dashboard 迁移", () => {
  it("全部范围纳入旧记录，七天范围不混入更早用量", async () => {
    rows.push({
      ...rows[0],
      id: 0,
      startedAt: new Date(Date.now() - 20 * 86400000).toISOString(),
      model: "older-model",
    });
    const user = userEvent.setup();
    render(<ManualApp initialWorkspace="overview" />);
    const metrics = await screen.findByLabelText("概览核心指标");
    await waitFor(() => expect(metrics).toHaveTextContent("120"));
    await user.click(screen.getByRole("combobox", { name: "概览时间范围" }));
    await user.click(screen.getByRole("option", { name: "全部" }));
    await waitFor(() => expect(metrics).toHaveTextContent("240"));
    expect(screen.getByRole("button", { name: /older-model/ })).toBeVisible();
    await user.click(screen.getByRole("combobox", { name: "概览时间范围" }));
    await user.click(screen.getByRole("option", { name: "最近 7 天" }));
    await waitFor(() => expect(metrics).toHaveTextContent("120"));
    expect(
      screen.queryByRole("button", { name: /older-model/ }),
    ).not.toBeInTheDocument();
  });
  it("入口默认折叠，概览只读真实 IPC，并正确区分失败与未知用量", async () => {
    render(<ManualApp initialWorkspace="overview" />);
    const metrics = await screen.findByLabelText("概览核心指标");
    await waitFor(() => expect(metrics).toHaveTextContent("50.0%"));
    expect(metrics).toHaveTextContent("1 次失败 · 1 次待确认");
    expect(metrics).toHaveTextContent("120");
    expect(screen.getByRole("button", { name: "展开侧栏" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(calls()).toEqual(["manual_snapshot", "manual_query_logs"]);
    expect(rpc.mock.calls[1][1]).toMatchObject({
      query: { page: 0, pageSize: 25, grouping: "model" },
    });
  });
  it("从异常进入日志会带入时间和失败条件，而不触发任何写入", async () => {
    const user = userEvent.setup();
    render(<ManualApp initialWorkspace="overview" />);
    await screen.findByText("1 次失败 · 1 次待确认");
    await user.click(screen.getByRole("button", { name: "查看日志" }));
    await screen.findByRole("table", { name: "网关调用日志" });
    await waitFor(() =>
      expect(rpc.mock.calls.at(-1)?.[1]).toMatchObject({
        query: { status: "failed", page: 0 },
      }),
    );
    expect(
      screen.getByRole("combobox", { name: "日志时间范围" }),
    ).toHaveTextContent("最近 24 小时");
    expect(
      calls().every((c) =>
        ["manual_snapshot", "manual_query_logs"].includes(c),
      ),
    ).toBe(true);
  });
  it("概览查询失败不显示私密异常内容，刷新可恢复且不启动网关", async () => {
    const user = userEvent.setup();
    const original = rpc.getMockImplementation()!;
    let fail = true;
    rpc.mockImplementation(async (command, args) => {
      if (command === "manual_query_logs" && fail)
        throw Error("PRIVATE_BACKEND_ERROR");
      return original(command, args);
    });
    render(<ManualApp initialWorkspace="overview" />);
    expect(await screen.findByRole("alert")).toHaveTextContent("读取失败");
    expect(screen.queryByText(/PRIVATE_BACKEND_ERROR/)).not.toBeInTheDocument();
    fail = false;
    await user.click(screen.getByRole("button", { name: "刷新概览" }));
    await screen.findByText("1 次失败 · 1 次待确认");
    expect(calls()).not.toContain("manual_gateway");
  });
  it("侧栏切换可用键盘操作，页面切换保持展开但重挂载默认收起", async () => {
    const user = userEvent.setup();
    const view = render(<ManualApp />);
    await screen.findByRole("heading", { name: "模型" });
    screen.getByRole("button", { name: "展开侧栏" }).focus();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("button", { name: "收起侧栏" })).toHaveFocus();
    await user.click(screen.getByRole("button", { name: "客户端" }));
    expect(screen.getByRole("button", { name: "收起侧栏" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    view.unmount();
    render(<ManualApp />);
    expect(screen.getByRole("button", { name: "展开侧栏" })).toBeVisible();
    expect(calls()).toEqual(["manual_snapshot", "manual_snapshot"]);
  });
  it("隐藏日志保留筛选但停止只读轮询，返回时继续读取", async () => {
    const view = render(<RequestLogs />);
    await screen.findByRole("table", { name: "网关调用日志" });
    const count = rpc.mock.calls.length;
    vi.useFakeTimers();
    view.rerender(<RequestLogs active={false} />);
    await act(async () => {
      vi.advanceTimersByTime(12000);
    });
    expect(rpc).toHaveBeenCalledTimes(count);
    view.rerender(<RequestLogs active />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(rpc).toHaveBeenCalledTimes(count + 1);
  });
  it("日志库不可用时，概览提供恢复入口且启动按钮禁用", async () => {
    snapshot.logStorage = {
      ...readyStorage,
      ready: false,
      message: "只读错误",
    };
    rpc.mockImplementation(async (command) =>
      command === "manual_snapshot"
        ? structuredClone(snapshot)
        : { storage: snapshot.logStorage },
    );
    render(<ManualApp initialWorkspace="overview" />);
    await screen.findByRole("heading", { name: "运行概览" });
    await waitFor(() =>
      expect(
        screen
          .getAllByRole("alert")
          .some((a) => a.textContent?.includes("日志存储不可用")),
      ).toBe(true),
    );
    expect(screen.getByRole("button", { name: "启动网关" })).toBeDisabled();
    expect(calls()).not.toContain("manual_retry_log_storage");
    expect(calls()).not.toContain("manual_gateway");
  });
});
