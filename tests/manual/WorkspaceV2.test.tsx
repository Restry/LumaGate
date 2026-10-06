import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { historyFixture } from "./history-fixture";
import ManualApp from "@/manual/ManualApp";
import {
  catalogEntries,
  initialCatalogView,
  sourceResults,
} from "@/manual/catalog-view";
import {
  testKey,
  type Group,
  type Snapshot,
  type TestResult,
} from "@/manual/types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: () => false }));
const rpc = vi.mocked(invoke);
let data: Snapshot;
function fixture(): Snapshot {
  const model = {
    id: "demo",
    enabled: true,
    image: false,
    tools: null,
    contextWindow: null,
  };
  const group: Group = {
    id: "demo-route",
    model,
    providerIds: ["a", "b"],
    providerNames: ["Example", "Backup"],
    protocol: "openai_chat",
    protocolMode: "auto",
    policy: { balance: "round_robin", failover: true, fallbacks: [] },
  };
  return {
    document: {
      revision: 1,
      policies: {},
      blockedModels: [],
      tests: {
        [testKey(group.id, "a")]: {
          success: true,
          latencyMs: 120,
          checkedAt: "2026-01-01T10:00:00Z",
          detail: "fixture passed",
        },
        [testKey(group.id, "b")]: {
          success: false,
          latencyMs: 130,
          checkedAt: "2026-01-01T10:00:00Z",
          detail: "HTTP 503 · fixture failure",
        },
      },
      providers: ["a", "b"].map((id, index) => ({
        id,
        name: index ? "Backup" : "Example",
        baseUrl: `https://${id}.example/v1`,
        keyEnv: "",
        enabled: true,
        protocol: "openai_chat",
        models: [model],
      })),
    },
    groups: [
      group,
      {
        ...group,
        id: "other-route",
        model: { ...model, id: "other" },
        providerIds: ["a"],
        providerNames: ["Example"],
      },
    ],
    status: { running: true },
    baseUrl: "http://127.0.0.1:15722",
  };
}
const commands = () => rpc.mock.calls.map(([command]) => command);
async function openModel(
  user: ReturnType<typeof userEvent.setup>,
  name = "demo",
) {
  await user.click(
    await screen.findByRole("button", { name: `查看模型 ${name}` }),
  );
}

beforeEach(() => {
  data = fixture();
  rpc.mockReset();
  rpc.mockImplementation(async (command) => {
    if (command === "manual_snapshot") return structuredClone(data);
    if (command === "manual_query_logs") return historyFixture([]);
    if (command === "manual_preview_sync")
      return {
        id: "fixture-plan",
        revision: 1,
        note: "保留默认模型",
        files: [
          {
            path: "/isolated/config.toml",
            beforeHash: "fixture-hash",
            changes: { profile: "cc_switch_manual" },
          },
        ],
      };
    if (command === "manual_apply_sync")
      return [
        {
          path: "/isolated/config.toml",
          backup: "/isolated/config.toml.backup",
        },
      ];
    return undefined;
  });
});

describe("模型工作台 V2", () => {
  it("展示和复制公开模型名，内部测试仍使用稳定路由 ID", async () => {
    data.groups[0].publicId = "demo";
    const user = userEvent.setup();
    render(<ManualApp />);
    await openModel(user);
    const copy = vi.spyOn(navigator.clipboard, "writeText");
    await user.click(screen.getByRole("button", { name: "复制模型 ID demo" }));
    expect(copy).toHaveBeenCalledWith("demo");
    await user.click(screen.getByText("能力与原始信息"));
    expect(screen.getByRole("region", { name: "模型详情" })).toHaveTextContent(
      "请求模型 ID",
    );
    expect(screen.getByRole("region", { name: "模型详情" })).toHaveTextContent(
      "兼容路由 ID",
    );
    await user.click(screen.getByRole("button", { name: "在 Agent 中使用" }));
    await user.click(screen.getByRole("button", { name: "复制模型 ID" }));
    expect(copy).toHaveBeenLastCalledWith("demo");
    expect(commands()).toEqual(["manual_snapshot"]);
  });

  it("仅 Copilot 的公开短名称不要求界面或调用者拼接命名空间", async () => {
    data.groups[0].publicId = "demo";
    data.groups[0].model = { ...data.groups[0].model, id: "copilot/demo" };
    const user = userEvent.setup();
    render(<ManualApp />);
    await openModel(user, "demo");
    const copy = vi.spyOn(navigator.clipboard, "writeText");
    await user.click(screen.getByRole("button", { name: "复制模型 ID demo" }));
    expect(copy).toHaveBeenCalledWith("demo");
    expect(screen.getByRole("heading", { name: "demo" })).toBeVisible();
    expect(commands()).toEqual(["manual_snapshot"]);
  });
  it("用户指定的上下文与输出值明确标记来源，查看不会写配置或测试模型", async () => {
    data.groups[0].model = {
      ...data.groups[0].model,
      contextWindow: 1_000_000,
      maxOutputTokens: 128_000,
      limitsSource: "user_override",
    };
    const before = structuredClone(data.document);
    const user = userEvent.setup();
    render(<ManualApp />);
    await openModel(user);
    await user.click(screen.getByText("能力与原始信息"));
    expect(screen.getByText("1,000,000")).toBeVisible();
    expect(screen.getByText("128,000")).toBeVisible();
    expect(screen.getByText(/上下文与最大输出按用户指定配置/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "查看元数据" }));
    expect(
      screen.getByRole("dialog", { name: "模型元数据" }),
    ).not.toHaveTextContent("1000000");
    expect(data.document).toEqual(before);
    expect(commands()).toEqual(["manual_snapshot"]);
  });

  it("未指定限额的模型继续显示未知和上游声明提示", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await openModel(user);
    await user.click(screen.getByText("能力与原始信息"));
    expect(
      screen.getByText("能力来自上游声明；未知不等于不支持。"),
    ).toBeVisible();
    expect(
      screen.queryByText(/上下文与最大输出按用户指定配置/),
    ).not.toBeInTheDocument();
    expect(commands()).toEqual(["manual_snapshot"]);
  });

  it("列表只展示摘要，打开详情才显示独立测试且不发请求", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    expect(await screen.findByText("1 通过 · 1 失败")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "测试 demo · Backup" }),
    ).not.toBeInTheDocument();
    await openModel(user);
    const list = screen.getByRole("list", { name: "demo 的来源测试" });
    expect(within(list).getByText("HTTP 503 · fixture failure")).toBeVisible();
    expect(within(list).getAllByRole("listitem")[0]).toHaveAttribute(
      "data-provider-id",
      "b",
    );
    expect(data.groups[0].providerIds).toEqual(["a", "b"]);
    expect(commands()).toEqual(["manual_snapshot"]);
  });

  it("搜索、详情和模型上下文在来源维护往返后保留", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByRole("button", { name: "查看模型 demo" });
    await user.type(
      screen.getByRole("searchbox", { name: "搜索模型" }),
      "demo",
    );
    await openModel(user);
    await user.click(screen.getByRole("button", { name: "编辑来源 Backup" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "取消" }));
    await user.click(screen.getByRole("button", { name: "返回刚才的模型" }));
    expect(screen.getByRole("searchbox", { name: "搜索模型" })).toHaveValue(
      "demo",
    );
    expect(screen.getByRole("region", { name: "模型详情" })).toHaveTextContent(
      "HTTP 503 · fixture failure",
    );
    expect(
      screen.queryByRole("button", { name: "查看模型 other" }),
    ).not.toBeInTheDocument();
    expect(commands()).toEqual(["manual_snapshot"]);
  });

  it("待检查和未测试按来源记录筛选，不把混合结果标为整体不可用", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByRole("button", { name: "查看模型 demo" });
    await user.click(screen.getByRole("tab", { name: /待检查/ }));
    expect(
      screen.getByRole("button", { name: "查看模型 demo" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "查看模型 other" }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: /含未测试/ }));
    expect(
      screen.getByRole("button", { name: "查看模型 other" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "查看模型 demo" }),
    ).not.toBeInTheDocument();
    expect(commands()).toEqual(["manual_snapshot"]);
  });

  it("测试更新使模型不再匹配筛选时，仍保留本次详情便于读取结果", async () => {
    rpc.mockImplementation(async (command) => {
      if (command === "manual_snapshot") return structuredClone(data);
      if (command === "manual_test") {
        const result = {
          success: true,
          latencyMs: 42,
          checkedAt: "2026-01-02",
          detail: "fixture success",
        };
        data.document.tests[testKey("other-route", "a")] = result;
        return result;
      }
    });
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByRole("button", { name: "查看模型 demo" });
    await user.click(screen.getByRole("tab", { name: /含未测试/ }));
    await openModel(user, "other");
    await user.click(
      screen.getByRole("button", { name: "测试 other · Example" }),
    );
    expect(await screen.findByText("上次通过 · 42 ms")).toBeVisible();
    expect(screen.getByRole("region", { name: "模型详情" })).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "查看模型 other" }),
    ).not.toBeInTheDocument();
    expect(
      commands().filter((command) => command === "manual_test"),
    ).toHaveLength(1);
  });

  it("来源测试期间复制可用且不重新读取快照，其他测试仍禁止重复发送", async () => {
    let resolveTest!: (result: TestResult) => void;
    rpc.mockImplementation(async (command) => {
      if (command === "manual_snapshot") return structuredClone(data);
      if (command === "manual_test")
        return new Promise<TestResult>((resolve) => {
          resolveTest = resolve;
        });
    });
    const user = userEvent.setup();
    render(<ManualApp />);
    await openModel(user);
    const copy = vi.spyOn(navigator.clipboard, "writeText");
    await user.click(
      screen.getByRole("button", { name: "测试 demo · Backup" }),
    );
    expect(
      screen.getByRole("button", { name: "测试 demo · Example" }),
    ).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "复制模型 ID demo" }));
    expect(copy).toHaveBeenCalledWith("demo-route");
    expect(commands()).toEqual(["manual_snapshot", "manual_test"]);
    await act(async () =>
      resolveTest({
        success: false,
        latencyMs: 10,
        checkedAt: "2026-01-01",
        detail: "fixture error",
      }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "测试 demo · Backup" }),
      ).toBeEnabled(),
    );
  });

  it("返回列表恢复键盘焦点到原模型", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await openModel(user);
    expect(screen.getByRole("heading", { name: "demo" })).toHaveFocus();
    await user.click(screen.getByRole("button", { name: "返回列表" }));
    expect(screen.getByRole("button", { name: "查看模型 demo" })).toHaveFocus();
  });

  it("手动 Endpoint 恢复自动时传 null，而不是伪造 auto 协议", async () => {
    data.groups[0].protocolMode = "manual";
    const user = userEvent.setup();
    render(<ManualApp />);
    await openModel(user);
    await user.click(screen.getByText("调用与路由设置"));
    await user.click(screen.getByRole("combobox", { name: "Endpoint demo" }));
    await user.click(screen.getByRole("option", { name: /自动/ }));
    await waitFor(() =>
      expect(rpc).toHaveBeenCalledWith("manual_set_model_protocol", {
        groupId: "demo-route",
        protocol: null,
        revision: 1,
      }),
    );
    expect(commands()).not.toContain("manual_test");
  });

  it("不一致的 Endpoint 保持不一致状态，不伪装成自动", async () => {
    data.groups[0].protocolMode = "mixed";
    const user = userEvent.setup();
    render(<ManualApp />);
    await openModel(user);
    await user.click(screen.getByText("调用与路由设置"));
    expect(
      screen.getByRole("combobox", { name: "Endpoint demo" }),
    ).toHaveTextContent("来源设置不一致");
    expect(commands()).toEqual(["manual_snapshot"]);
  });

  it("后端读取失败不是空目录，并提供重试", async () => {
    rpc.mockRejectedValueOnce(new Error("fixture offline"));
    const user = userEvent.setup();
    render(<ManualApp />);
    expect(
      await screen.findByRole("heading", { name: "无法读取本机网关状态" }),
    ).toBeVisible();
    expect(
      screen.queryByText("接入你的第一个模型来源"),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("网关已停止")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重新连接" }));
    expect(
      await screen.findByRole("button", { name: "查看模型 demo" }),
    ).toBeVisible();
    expect(commands()).toEqual(["manual_snapshot", "manual_snapshot"]);
  });

  it("失败记录过滤不读取旧组级结果，屏蔽后不计入活动状态", () => {
    delete data.document.tests[testKey("demo-route", "b")];
    data.document.tests["demo-route"] = {
      success: false,
      latencyMs: 0,
      checkedAt: "old",
      detail: "legacy",
    };
    expect(sourceResults(data.groups[0], data.document.tests)).toEqual({
      passed: 1,
      failed: 0,
      untested: 1,
    });
    data.document.blockedModels = ["demo"];
    const entries = catalogEntries(data, initialCatalogView);
    expect(entries.counts).toEqual({
      all: 1,
      failed: 0,
      untested: 1,
      blocked: 1,
    });
    expect(entries.matches.map((group) => group.id)).toEqual(["other-route"]);
  });
});

describe("接入与使用的下一步", () => {
  it("明确点击保存并获取才拉取；获取失败保留已保存来源并给出恢复入口", async () => {
    rpc.mockImplementation(async (command, raw) => {
      if (command === "manual_snapshot") return structuredClone(data);
      if (command === "manual_save") {
        data.document = structuredClone(
          (raw as { document: Snapshot["document"] }).document,
        );
        return [];
      }
      if (command === "manual_discover") throw new Error("fixture timeout");
    });
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByRole("button", { name: "查看模型 demo" });
    await user.click(screen.getByRole("button", { name: "Provider" }));
    await user.click(screen.getByRole("button", { name: "添加 Provider" }));
    await user.type(screen.getByLabelText("名称"), "New source");
    await user.type(
      screen.getByLabelText("API 地址"),
      "https://new.example/v1",
    );
    await user.click(screen.getByRole("button", { name: "保存并获取模型" }));
    const source = await screen.findByRole("region", {
      name: "来源 New source",
    });
    expect(
      within(source).getByRole("button", { name: "刷新模型" }),
    ).toBeEnabled();
    expect(commands()).toEqual([
      "manual_snapshot",
      "manual_save",
      "manual_discover",
      "manual_snapshot",
    ]);
    expect(data.document.providers).toHaveLength(3);
  });

  it("同步结果在切换工作区后保留，Codex 显示真实命令但不自动启动", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await openModel(user);
    await user.click(screen.getByRole("button", { name: "在 Agent 中使用" }));
    await user.click(screen.getByRole("tab", { name: "Codex" }));
    const copy = vi.spyOn(navigator.clipboard, "writeText");
    await user.click(screen.getByRole("button", { name: "复制网关启动命令" }));
    expect(copy).toHaveBeenCalledWith("codex --profile cc_switch_manual");
    await user.click(screen.getByRole("button", { name: "预览同步" }));
    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByText("受管字段的写入摘要（不是完整文件差异）"),
    ).toBeInTheDocument();
    await user.click(
      within(dialog).getByRole("button", { name: "确认写入配置" }),
    );
    expect(
      await screen.findByRole("region", { name: "Codex 同步结果" }),
    ).toHaveTextContent("本次会话已同步");
    await user.click(screen.getByRole("button", { name: "模型" }));
    await user.click(screen.getByRole("button", { name: "客户端" }));
    expect(
      screen.getByRole("region", { name: "Codex 同步结果" }),
    ).toHaveTextContent("不代表客户端已连接");
    await user.click(screen.getByText("查看写入文件与备份"));
    expect(
      screen.getByText("备份：/isolated/config.toml.backup"),
    ).toBeVisible();
    expect(commands()).not.toContain("manual_launch");
    expect(rpc).toHaveBeenCalledWith("manual_preview_sync", { agent: "codex" });
    expect(rpc).toHaveBeenCalledWith("manual_apply_sync", {
      planId: "fixture-plan",
    });
  });
});
