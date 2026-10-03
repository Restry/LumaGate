import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { historyFixture } from "./history-fixture";
import ManualApp from "@/manual/ManualApp";
import {
  compatible,
  reasoningLabel,
  testKey,
  type Snapshot,
} from "@/manual/types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const rpc = vi.mocked(invoke);
const snapshot: Snapshot = {
  document: {
    revision: 1,
    providers: [
      {
        id: "a",
        name: "Example",
        baseUrl: "https://example.com/v1",
        keyEnv: "EXAMPLE_KEY",
        enabled: true,
        protocol: "openai_chat",
        models: [
          {
            id: "demo",
            image: false,
            tools: null,
            contextWindow: null,
            enabled: true,
          },
        ],
      },
    ],
    policies: {},
    tests: {},
  },
  groups: [
    {
      id: "demo--hash",
      model: {
        id: "demo",
        image: false,
        tools: null,
        contextWindow: null,
        enabled: true,
      },
      protocol: "openai_chat",
      providerIds: ["a"],
      providerNames: ["Example"],
      policy: { balance: "round_robin", failover: true, fallbacks: [] },
    },
  ],
  status: { running: false },
  baseUrl: "http://127.0.0.1:15722",
};
function calls() {
  return rpc.mock.calls.map(([command]) => command);
}
async function openModel(user: ReturnType<typeof userEvent.setup>) {
  await user.click(
    await screen.findByRole("button", { name: "查看模型 demo" }),
  );
}

beforeEach(() => {
  rpc.mockReset();
  rpc.mockImplementation(async (command) => {
    if (command === "manual_snapshot") return structuredClone(snapshot);
    if (command === "manual_preview_sync")
      return {
        id: "plan-1",
        revision: 1,
        note: "保留默认模型",
        files: [
          {
            path: "/temporary/.pi/agent/models.json",
            beforeHash: null,
            changes: { providers: {} },
          },
        ],
      };
    if (command === "manual_query_logs") return historyFixture([]);
    if (command === "manual_apply_sync")
      return [{ path: "/temporary/.pi/agent/models.json", backup: null }];
    if (command === "manual_test")
      return {
        success: true,
        detail: "单次测试通过",
        checkedAt: "2026-01-01",
        latencyMs: 12,
      };
    return undefined;
  });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("手动网关操作边界", () => {
  it("日志库不可用仍展示模型配置与恢复入口，不伪造空应用", async () => {
    const storage = {
      ready: false,
      path: "/isolated/logs/requests.sqlite3",
      message: "日志库不可用，历史文件未被删除",
    };
    rpc.mockImplementation(async (command) =>
      command === "manual_snapshot"
        ? { ...structuredClone(snapshot), logStorage: storage }
        : command === "manual_query_logs"
          ? { storage }
          : undefined,
    );
    const user = userEvent.setup();
    render(<ManualApp />);
    expect(
      await screen.findByRole("button", { name: "查看模型 demo" }),
    ).toBeVisible();
    await user.click(screen.getByRole("button", { name: "查看日志存储" }));
    expect(
      await screen.findByRole("heading", { name: "日志存储需要处理" }),
    ).toBeVisible();
    expect(calls()).toEqual(["manual_snapshot", "manual_query_logs"]);
  });
  it("打开界面只读取快照，不自动发现、测试、启动或同步", async () => {
    render(<ManualApp />);
    expect(await screen.findByText("demo")).toBeInTheDocument();
    expect(
      within(screen.getByRole("list", { name: "模型列表" })).getByText(
        "1 未测试",
      ),
    ).toBeInTheDocument();
    expect(calls()).toEqual(["manual_snapshot"]);
  });

  it("日志导航只读取日志，不启动网关或写入配置", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByText("demo");
    await user.click(screen.getByRole("button", { name: "调用日志" }));
    expect(
      await screen.findByRole("heading", { name: "调用日志" }),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("请求网关后，记录会出现在这里。"),
    ).toBeInTheDocument();
    expect(calls()).toEqual(["manual_snapshot", "manual_query_logs"]);
  });

  it("模型行支持手动设置 Endpoint，且不触发测试或同步", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByText("demo");
    await openModel(user);
    await user.click(screen.getByText("调用与路由设置"));
    const trigger = screen.getByRole("combobox", { name: "Endpoint demo" });
    await user.click(trigger);
    await user.click(
      await screen.findByRole("option", { name: "OpenAI Responses" }),
    );
    await waitFor(() =>
      expect(rpc).toHaveBeenCalledWith("manual_set_model_protocol", {
        groupId: "demo--hash",
        protocol: "openai_responses",
        revision: 1,
      }),
    );
    expect(calls()).not.toContain("manual_test");
    expect(calls()).not.toContain("manual_apply_sync");
  });

  it("启动网关只调用服务启动入口", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByText("demo");
    await user.click(screen.getByRole("button", { name: "启动网关" }));
    await waitFor(() =>
      expect(rpc).toHaveBeenCalledWith("manual_gateway", { running: true }),
    );
    expect(calls()).not.toContain("manual_apply_sync");
    expect(calls()).not.toContain("manual_test");
  });

  it.each(["Pi", "Claude Code", "Codex"])(
    "启动 %s 不同步或测试模型",
    async (name) => {
      const user = userEvent.setup();
      render(<ManualApp />);
      await screen.findByText("demo");
      await user.click(screen.getByRole("button", { name: "客户端" }));
      await user.click(screen.getByRole("tab", { name }));
      await user.click(screen.getByRole("button", { name: `启动 ${name}` }));
      await waitFor(() => expect(calls()).toContain("manual_launch"));
      expect(calls()).not.toContain("manual_preview_sync");
      expect(calls()).not.toContain("manual_apply_sync");
      expect(calls()).not.toContain("manual_test");
    },
  );

  it("预览和取消都不写配置，确认后只提交预览令牌", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByText("demo");
    await user.click(screen.getByRole("button", { name: "客户端" }));
    await user.click(screen.getAllByRole("button", { name: "预览同步" })[0]);
    const dialog = await screen.findByRole("dialog");
    expect(calls()).not.toContain("manual_apply_sync");
    await user.click(
      within(dialog).getByRole("button", { name: "取消，不写入" }),
    );
    expect(calls()).not.toContain("manual_apply_sync");
    await user.click(screen.getAllByRole("button", { name: "预览同步" })[0]);
    await user.click(
      within(await screen.findByRole("dialog")).getByRole("button", {
        name: "确认写入配置",
      }),
    );
    await waitFor(() =>
      expect(rpc).toHaveBeenCalledWith("manual_apply_sync", {
        planId: "plan-1",
      }),
    );
    expect(calls().filter((c) => c === "manual_apply_sync")).toHaveLength(1);
    expect(calls()).not.toContain("manual_launch");
  });

  it("单模型测试点击即执行，不弹确认且不产生配置写入", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await openModel(user);
    await user.click(
      screen.getByRole("button", { name: "测试 demo · Example" }),
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await waitFor(() =>
      expect(rpc).toHaveBeenCalledWith("manual_test", {
        groupId: "demo--hash",
        providerId: "a",
      }),
    );
    expect(calls().filter((c) => c === "manual_test")).toHaveLength(1);
    expect(calls()).not.toContain("manual_save");
    expect(calls()).not.toContain("manual_apply_sync");
  });

  it("添加 Provider 不触发自动发现或 live 配置同步", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByText("demo");
    await user.click(screen.getByRole("button", { name: "Provider" }));
    await user.click(screen.getByRole("button", { name: "添加 Provider" }));
    fireEvent.change(screen.getByLabelText("名称"), {
      target: { value: "Second" },
    });
    fireEvent.change(screen.getByLabelText("API 地址"), {
      target: { value: "https://second.example/v1" },
    });
    expect(screen.queryByLabelText(/模型 ID/)).not.toBeInTheDocument();
    expect(screen.getByText("填写连接地址与 API Key。")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "保存，不同步" }));
    await waitFor(() => expect(calls()).toContain("manual_save"));
    expect(calls()).not.toContain("manual_discover");
    expect(calls()).not.toContain("manual_apply_sync");
  });
});

describe("简洁反馈与说明", () => {
  it("帮助不占据默认工作区，显式进入帮助页不触发后端操作", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByText("demo");
    expect(screen.queryByText(/GPT 5.5 及以上默认/)).not.toBeInTheDocument();
    expect(screen.queryByText("你决定每一次更改")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "帮助" }));
    expect(await screen.findByRole("heading", { name: "帮助" })).toBeVisible();
    expect(screen.getByText(/GPT 5.5 及以上默认/)).toBeVisible();
    expect(calls()).toEqual(["manual_snapshot"]);
    await user.click(screen.getByRole("button", { name: "模型" }));
    expect(
      screen.queryByRole("heading", { name: "帮助" }),
    ).not.toBeInTheDocument();
  });

  it("重复点击不会重复请求，失败使用顶部错误气泡", async () => {
    let finish: (value: unknown) => void = () => {};
    const original = rpc.getMockImplementation()!;
    rpc.mockImplementation((command, args) =>
      command === "manual_test"
        ? new Promise((resolve) => {
            finish = resolve;
          })
        : original(command, args),
    );
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByText("demo");
    await openModel(user);
    const button = screen.getByRole("button", { name: "测试 demo · Example" });
    await user.dblClick(button);
    expect(button).toBeDisabled();
    expect(calls().filter((name) => name === "manual_test")).toHaveLength(1);
    await act(async () => {
      finish({ success: false, detail: "上游返回 HTTP 503", latencyMs: 10 });
    });
    const message = await screen.findByText(
      "demo · Example：上游返回 HTTP 503",
    );
    expect(message.closest("[data-sonner-toast]")).toHaveAttribute(
      "data-type",
      "error",
    );
    expect(document.querySelector("[data-sonner-toaster]")).toHaveAttribute(
      "data-y-position",
      "top",
    );
    expect(button).not.toBeDisabled();
  });

  it("成功气泡自动消失", async () => {
    render(<ManualApp />);
    await screen.findByText("demo");
    fireEvent.click(screen.getByRole("button", { name: "查看模型 demo" }));
    vi.useFakeTimers();
    await act(async () => {
      fireEvent.click(
        screen.getByRole("button", { name: "测试 demo · Example" }),
      );
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(100);
    });
    expect(
      screen.getByText("demo · Example：测试通过 · 12 ms"),
    ).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    expect(
      screen.queryByText("demo · Example：测试通过 · 12 ms"),
    ).not.toBeInTheDocument();
  });
});

describe("来源测试、模态与屏蔽", () => {
  it("显示已知图文架构，每个来源独立测试且旧组结果不冒充来源结果", async () => {
    const data = structuredClone(snapshot);
    const group = data.groups[0];
    group.providerIds.push("b");
    group.providerNames.push("Backup");
    const backup = structuredClone(data.document.providers[0]);
    backup.id = "b";
    backup.name = "Backup";
    data.document.providers.push(backup);
    data.document.providers[0].models[0].inputModalities = ["image", "text"];
    data.document.providers[0].models[0].outputModalities = ["text"];
    group.model.inputModalities = ["image", "text"];
    group.model.outputModalities = ["text"];
    data.document.tests[group.id] = {
      success: true,
      latencyMs: 99,
      checkedAt: "old",
      detail: "旧组结果",
    };
    rpc.mockImplementation(async (command, args) => {
      if (command === "manual_snapshot") return structuredClone(data);
      if (command === "manual_test" && args && "providerId" in args) {
        const id = args.providerId as string;
        const result = {
          success: id === "a",
          latencyMs: 12,
          checkedAt: "now",
          detail: id === "a" ? "测试通过" : "HTTP 503",
          providerId: id,
        };
        data.document.tests[testKey(group.id, id)] = result;
        return result;
      }
      return undefined;
    });
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByText("demo");
    await openModel(user);
    expect(
      screen.getByText("输入：文本、图片 · 输出：文本"),
    ).toBeInTheDocument();
    const list = screen.getByRole("list", { name: "demo 的来源测试" });
    expect(within(list).getAllByText("尚未测试")).toHaveLength(2);
    await user.click(
      within(list).getByRole("button", { name: "测试 demo · Example" }),
    );
    await waitFor(() =>
      expect(within(list).getByText("上次通过 · 12 ms")).toBeInTheDocument(),
    );
    expect(within(list).getAllByText("尚未测试")).toHaveLength(1);
    await user.click(
      within(list).getByRole("button", { name: "测试 demo · Backup" }),
    );
    await waitFor(() =>
      expect(within(list).getByText("上次测试失败")).toBeInTheDocument(),
    );
    expect(within(list).getByText("HTTP 503")).toBeVisible();
    expect(within(list).getByText("上次通过 · 12 ms")).toBeInTheDocument();
    expect(
      rpc.mock.calls
        .filter(([name]) => name === "manual_test")
        .map(([, args]) => args),
    ).toEqual([
      { groupId: group.id, providerId: "a" },
      { groupId: group.id, providerId: "b" },
    ]);
  });

  it("屏蔽移入已屏蔽视图，恢复不改变来源选择且不触发测试同步", async () => {
    const data = structuredClone(snapshot);
    rpc.mockImplementation(async (command, args) => {
      if (command === "manual_snapshot") return structuredClone(data);
      if (command === "manual_set_model_blocked" && args && "blocked" in args) {
        data.document.blockedModels = args.blocked ? ["demo"] : [];
        data.groups[0].blocked = !!args.blocked;
        data.document.revision++;
      }
      return undefined;
    });
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByText("demo");
    await openModel(user);
    await user.click(screen.getByRole("button", { name: "屏蔽 demo" }));
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "测试 demo · Example" }),
      ).not.toBeInTheDocument(),
    );
    const blockedTab = screen.getByRole("tab", { name: /已屏蔽/ });
    expect(blockedTab).toHaveAttribute("aria-selected", "false");
    expect(rpc).toHaveBeenCalledWith("manual_set_model_blocked", {
      modelId: "demo",
      blocked: true,
      revision: 1,
    });
    await user.click(blockedTab);
    await user.click(screen.getByRole("button", { name: "恢复 demo" }));
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "恢复 demo" }),
      ).not.toBeInTheDocument(),
    );
    await user.click(screen.getByRole("tab", { name: /全部模型/ }));
    await openModel(user);
    expect(
      screen.getByRole("button", { name: "测试 demo · Example" }),
    ).toBeInTheDocument();
    expect(data.document.providers[0].models[0].enabled).toBe(true);
    expect(calls()).not.toContain("manual_save");
    expect(calls()).not.toContain("manual_test");
    expect(calls()).not.toContain("manual_apply_sync");
  });
});

describe("Provider 模型选择", () => {
  function withModels() {
    const data = structuredClone(snapshot);
    const first = data.document.providers[0].models[0];
    data.document.providers[0].models = [
      first,
      { ...structuredClone(first), id: "demo-mini" },
      { ...structuredClone(first), id: "kimi-k3" },
    ];
    data.groups = data.document.providers[0].models.map((model) => ({
      ...structuredClone(snapshot.groups[0]),
      id: `${model.id}--hash`,
      model,
    }));
    rpc.mockResolvedValueOnce(data);
  }
  async function openEditor() {
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByText("demo");
    await user.click(screen.getByRole("button", { name: "Provider" }));
    await user.click(
      screen.getByRole("button", { name: "选择 Example 的模型" }),
    );
    return { user, dialog: await screen.findByRole("dialog") };
  }
  it("整行、开关和空格都可切换，保存前不写入", async () => {
    withModels();
    const { user, dialog } = await openEditor();
    const control = within(dialog).getByRole("switch", {
      name: "启用模型 demo",
    });
    expect(control).toBeChecked();
    await user.click(control.closest("label")!);
    expect(control).not.toBeChecked();
    await user.click(control);
    expect(control).toBeChecked();
    control.focus();
    await user.keyboard(" ");
    expect(control).not.toBeChecked();
    expect(calls()).toEqual(["manual_snapshot"]);
    await user.click(
      within(dialog).getByRole("button", { name: "保存模型选择" }),
    );
    await waitFor(() => expect(calls()).toContain("manual_save"));
    const args = rpc.mock.calls.find(([name]) => name === "manual_save")?.[1];
    expect(args).toMatchObject({
      document: {
        providers: [
          {
            models: [
              expect.objectContaining({ id: "demo", enabled: false }),
              expect.objectContaining({ id: "demo-mini", enabled: true }),
              expect.objectContaining({ id: "kimi-k3", enabled: true }),
            ],
          },
        ],
      },
    });
    expect(calls()).not.toContain("manual_discover");
    expect(calls()).not.toContain("manual_test");
    expect(calls()).not.toContain("manual_apply_sync");
  });
  it("搜索与批量开关只影响筛选结果，Enter 不提交", async () => {
    withModels();
    const { user, dialog } = await openEditor();
    const search = within(dialog).getByRole("searchbox", {
      name: "搜索 Provider 模型",
    });
    await user.type(search, "DEMO{Enter}");
    expect(
      within(dialog).queryByRole("switch", { name: "启用模型 kimi-k3" }),
    ).not.toBeInTheDocument();
    await user.click(
      within(dialog).getByRole("button", { name: "停用筛选结果" }),
    );
    expect(
      within(dialog).getByRole("switch", { name: "启用模型 demo" }),
    ).not.toBeChecked();
    await user.clear(search);
    await user.type(search, "mini");
    await user.click(
      within(dialog).getByRole("button", { name: "启用筛选结果" }),
    );
    await user.clear(search);
    await user.type(search, "missing");
    expect(
      within(dialog).getByRole("button", { name: "启用筛选结果" }),
    ).toBeDisabled();
    expect(
      within(dialog).getByRole("button", { name: "停用筛选结果" }),
    ).toBeDisabled();
    await user.clear(search);
    expect(
      within(dialog).getByRole("switch", { name: "启用模型 kimi-k3" }),
    ).toBeChecked();
    expect(
      within(dialog).getByRole("switch", { name: "启用模型 demo-mini" }),
    ).toBeChecked();
    expect(
      within(dialog).getByRole("switch", { name: "启用模型 demo" }),
    ).not.toBeChecked();
    expect(calls()).toEqual(["manual_snapshot"]);
  });
  it("取消不保存选择，再次打开恢复原状态", async () => {
    withModels();
    const { user, dialog } = await openEditor();
    await user.click(
      within(dialog).getByRole("button", { name: "停用筛选结果" }),
    );
    await user.click(within(dialog).getByRole("button", { name: "取消" }));
    await user.click(
      screen.getByRole("button", { name: "选择 Example 的模型" }),
    );
    const controls = within(await screen.findByRole("dialog")).getAllByRole(
      "switch",
    );
    expect(controls).toHaveLength(3);
    controls.forEach((control) => expect(control).toBeChecked());
    expect(calls()).toEqual(["manual_snapshot"]);
  });
  it("没有目录时提示先拉取，不自动请求模型", async () => {
    const data = structuredClone(snapshot);
    data.document.providers[0].models = [];
    data.groups = [];
    rpc.mockResolvedValueOnce(data);
    const user = userEvent.setup();
    render(<ManualApp />);
    expect(
      await screen.findByText("来源已保存，下一步获取模型"),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Provider" }));
    expect(
      screen.getByText("下一步：拉取模型目录，再选择启用范围。"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "选择 Example 的模型" }),
    ).toBeDisabled();
    expect(calls()).toEqual(["manual_snapshot"]);
  });
});

describe("直接填写密钥", () => {
  it("新增默认使用密码框，密钥不混入 Provider 文档", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByText("demo");
    await user.click(screen.getByRole("button", { name: "Provider" }));
    await user.click(screen.getByRole("button", { name: "添加 Provider" }));
    const input = screen.getByLabelText("API Key");
    expect(input).toHaveAttribute("type", "password");
    expect(screen.queryByLabelText("环境变量名称")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("名称"), {
      target: { value: "Direct" },
    });
    fireEvent.change(screen.getByLabelText("API 地址"), {
      target: { value: "https://direct.example/v1" },
    });
    fireEvent.change(input, { target: { value: "fixture-direct-key-value" } });
    await user.click(screen.getByRole("button", { name: "保存，不同步" }));
    await waitFor(() => expect(calls()).toContain("manual_save"));
    const args = rpc.mock.calls.find(([name]) => name === "manual_save")?.[1];
    expect(args).toMatchObject({
      keyUpdate: { action: "set", value: "fixture-direct-key-value" },
    });
    expect(
      JSON.stringify(args && "document" in args ? args.document : undefined),
    ).not.toContain("fixture-direct-key-value");
    expect(calls()).not.toContain("manual_discover");
    expect(calls()).not.toContain("manual_apply_sync");
  });

  it("编辑已存密钥不会回显，留空不发送替换指令", async () => {
    const data = structuredClone(snapshot);
    data.document.providers[0].keyEnv = "";
    data.document.providers[0].keyRef = "00000000-0000-4000-8000-000000000001";
    rpc.mockResolvedValueOnce(data);
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByText("demo");
    await user.click(screen.getByRole("button", { name: "Provider" }));
    expect(screen.getByText(/API Key 已保存/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "编辑来源 Example" }));
    expect(screen.getByLabelText("API Key")).toHaveValue("");
    expect(screen.getByLabelText("API Key")).toHaveAttribute(
      "placeholder",
      "已保存，留空保留原密钥",
    );
    expect(calls()).toEqual(["manual_snapshot"]);
    await user.click(screen.getByRole("button", { name: "保存，不同步" }));
    await waitFor(() => expect(calls()).toContain("manual_save"));
    const args = rpc.mock.calls.find(([name]) => name === "manual_save")?.[1];
    expect(
      args && "keyUpdate" in args ? args.keyUpdate : undefined,
    ).toBeUndefined();
  });

  it("取消会清空尚未保存的密钥输入", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByText("demo");
    await user.click(screen.getByRole("button", { name: "Provider" }));
    await user.click(screen.getByRole("button", { name: "添加 Provider" }));
    fireEvent.change(screen.getByLabelText("API Key"), {
      target: { value: "fixture-discard" },
    });
    await user.click(screen.getByRole("button", { name: "取消" }));
    await user.click(screen.getByRole("button", { name: "添加 Provider" }));
    expect(screen.getByLabelText("API Key")).toHaveValue("");
    expect(calls()).not.toContain("manual_save");
  });

  it("旧环境变量方式可以手动切换到直接输入", async () => {
    const user = userEvent.setup();
    render(<ManualApp />);
    await screen.findByText("demo");
    await user.click(screen.getByRole("button", { name: "Provider" }));
    await user.click(screen.getByRole("button", { name: "编辑来源 Example" }));
    expect(screen.getByLabelText("环境变量名称")).toHaveValue("EXAMPLE_KEY");
    await user.click(
      screen.getByRole("checkbox", { name: "使用环境变量，替代直接填写密钥" }),
    );
    fireEvent.change(screen.getByLabelText("API Key"), {
      target: { value: "fixture-replace-env" },
    });
    await user.click(screen.getByRole("button", { name: "保存，不同步" }));
    await waitFor(() => expect(calls()).toContain("manual_save"));
    const args = rpc.mock.calls.find(([name]) => name === "manual_save")?.[1];
    expect(args).toMatchObject({
      keyUpdate: { action: "set", value: "fixture-replace-env" },
      document: { providers: [expect.objectContaining({ keyEnv: "" })] },
    });
  });
});

describe("保守模型元数据", () => {
  it("未知能力不变成支持或不支持，层级按常见顺序展示", () => {
    const model = snapshot.groups[0].model;
    expect(reasoningLabel(model)).toBe("未知");
    expect(reasoningLabel({ ...model, reasoning: false })).toBe("不支持");
    expect(reasoningLabel({ ...model, reasoning: true })).toBe(
      "支持（层级未知）",
    );
    expect(
      reasoningLabel({ ...model, reasoningLevels: ["high", "low", "max"] }),
    ).toBe("low / high / max");
  });
  it("不允许能力不同的模型进入后备选择", () => {
    const a = snapshot.groups[0];
    expect(compatible(a, { ...a, id: "b" })).toBe(true);
    expect(compatible(a, { ...a, id: "b", protocol: "anthropic" })).toBe(false);
    expect(
      compatible(a, { ...a, id: "b", model: { ...a.model, image: true } }),
    ).toBe(false);
    expect(
      compatible(a, {
        ...a,
        id: "b",
        model: { ...a.model, reasoningLevels: ["high"] },
      }),
    ).toBe(false);
    expect(
      compatible(a, {
        ...a,
        id: "b",
        model: { ...a.model, maxOutputTokens: 64000 },
      }),
    ).toBe(false);
  });

  it("展示上下文、输出与思考元数据，查看原始条目不发请求", async () => {
    const data = structuredClone(snapshot);
    const metadata = { id: "demo", vendor_extension: "extended-field" };
    const model = {
      ...data.groups[0].model,
      contextWindow: 400000,
      maxOutputTokens: 128000,
      reasoning: true,
      reasoningLevels: ["high", "low"],
      metadata,
    };
    data.groups[0].model = model;
    data.document.providers[0].models = [model];
    rpc.mockResolvedValueOnce(data);
    const user = userEvent.setup();
    render(<ManualApp />);
    await openModel(user);
    await user.click(screen.getByText("能力与原始信息"));
    const detail = screen.getByRole("region", { name: "模型详情" });
    expect(within(detail).getByText("400,000")).toBeInTheDocument();
    expect(within(detail).getByText("128,000")).toBeInTheDocument();
    expect(within(detail).getByText("low / high")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "查看元数据" }));
    expect(
      within(screen.getByRole("dialog")).getByText(/extended-field/),
    ).toBeInTheDocument();
    expect(calls()).toEqual(["manual_snapshot"]);
  });
});
