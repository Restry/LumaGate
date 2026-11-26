import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import ManualApp from "@/manual/ManualApp";
import type { Snapshot } from "@/manual/types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  isTauri: () => false,
}));

const rpc = vi.mocked(invoke);
let data: Snapshot;

beforeEach(() => {
  data = {
    document: {
      revision: 1,
      providers: [],
      policies: {},
      tests: {},
      blockedModels: [],
    },
    groups: [],
    status: { running: true },
    baseUrl: "http://127.0.0.1:15722",
    codexAuth: {
      connected: false,
      email: null,
      plan: null,
      accountId: null,
      source: null,
      localAvailable: true,
      localEmail: "subscriber@example.test",
      message: null,
    },
    agents: {
      codex: {
        connected: false,
        mode: "other",
        currentBaseUrl: "http://127.0.0.1:19999/v1",
        currentModel: "gpt-5.4",
        catalogPath: "/tmp/magpie-desktop-models.json",
        catalogOwner: "magpie",
        catalogModels: ["gpt-5.4", "gpt-5.3-codex"],
        autoSync: false,
        canRestore: false,
      },
      claude: { connected: false, currentBaseUrl: null, autoSync: false },
      pi: { connected: false, currentBaseUrl: null, autoSync: false },
    },
  };
  rpc.mockReset();
  rpc.mockImplementation(async (command, args) => {
    if (command === "manual_snapshot") return structuredClone(data);
    if (command === "manual_codex_status") return structuredClone(data.codexAuth);
    if (command === "manual_codex_import_local") {
      data.codexAuth = {
        connected: true,
        email: "subscriber@example.test",
        plan: "prolite",
        accountId: "acct-fixture-01",
        source: "local",
        localAvailable: true,
        localEmail: "subscriber@example.test",
        message: null,
      };
      data.document.providers.push({
        id: "codex-subscription",
        name: "Codex 订阅",
        baseUrl: "https://chatgpt.com/backend-api/codex",
        keyEnv: "",
        codex: {
          accountId: "acct-fixture-01",
          grantId: "grant-fixture",
        },
        protocol: "openai_responses",
        enabled: true,
        models: [],
      });
      data.document.revision++;
      return {
        providerId: "codex-subscription",
        email: "subscriber@example.test",
        plan: "prolite",
      };
    }
    if (command === "manual_discover") {
      const p = data.document.providers.find(
        (x) => x.id === "codex-subscription",
      )!;
      p.models = [
        {
          id: "codex/gpt-5.4",
          name: "gpt-5.4",
          enabled: true,
          image: true,
          tools: true,
          contextWindow: 1050000,
          protocolOverride: "openai_responses",
        },
        {
          id: "codex/gpt-5.3-codex",
          name: "gpt-5.3-codex",
          enabled: true,
          image: true,
          tools: true,
          contextWindow: 400000,
          protocolOverride: "openai_responses",
        },
      ];
      data.groups = p.models.map((m) => ({
        id: `${m.name!}--codex`,
        publicId: m.name!,
        model: m,
        modelIds: [m.id],
        protocol: "openai_responses",
        providerIds: [p.id],
        providerNames: [p.name],
        policy: { balance: "round_robin", failover: true, fallbacks: [] },
      }));
      data.document.revision++;
      return p.models.length;
    }
    if (command === "manual_set_auto_sync") {
      const { agent, enabled } = args as { agent: "codex"; enabled: boolean };
      data.agents![agent].autoSync = enabled;
      return true;
    }
    if (command === "manual_match_codex_models") {
      return 2;
    }
    if (command === "manual_set_codex_default_model") {
      const { modelId } = args as { modelId: string };
      data.agents!.codex.currentModel = modelId;
      return undefined;
    }
    return undefined;
  });
});

describe("Codex 订阅接入与客户端同步", () => {
  it("可导入本机 Codex 登录并拉取模型，且不要求手填 API Key", async () => {
    const user = userEvent.setup();
    render(<ManualApp initialWorkspace="sources" />);
    const connect = await screen.findByRole("button", {
      name: /接入 Codex 订阅/,
    });
    expect(connect).toHaveTextContent("本机已登录");
    await user.click(connect);
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("subscriber@example.test");
    await user.click(
      within(dialog).getByRole("button", { name: "一键导入" }),
    );
    await waitFor(() =>
      expect(rpc).toHaveBeenCalledWith("manual_codex_import_local"),
    );
    await waitFor(() =>
      expect(rpc).toHaveBeenCalledWith("manual_discover", {
        providerId: "codex-subscription",
      }),
    );
    expect(
      await screen.findByRole("button", { name: "查看模型 gpt-5.4" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "查看模型 gpt-5.3-codex" }),
    ).toBeVisible();
  });

  it("客户端页可按本机 Codex 目录匹配模型、切换默认模型并开启自动更新", async () => {
    // 先预置一个已同步的 Codex 订阅源
    await rpc("manual_codex_import_local");
    await rpc("manual_discover", { providerId: "codex-subscription" });
    rpc.mockClear();

    const user = userEvent.setup();
    render(<ManualApp initialWorkspace="agents" />);
    await user.click(await screen.findByRole("button", { name: "展开 Codex" }));
    expect(
      await screen.findByText(/检测到本机 Codex 当前配置了/),
    ).toBeVisible();

    await user.click(
      screen.getByRole("button", { name: "按本机列表匹配启用" }),
    );
    await waitFor(() =>
      expect(rpc).toHaveBeenCalledWith("manual_match_codex_models", {
        modelIds: ["gpt-5.4", "gpt-5.3-codex"],
        revision: 3,
      }),
    );

    const autoSyncSwitch = screen.getByRole("switch", {
      name: "Codex 自动更新可用模型",
    });
    await user.click(autoSyncSwitch);
    await waitFor(() =>
      expect(rpc).toHaveBeenCalledWith("manual_set_auto_sync", {
        agent: "codex",
        enabled: true,
      }),
    );

    const setDefault = screen.getByRole("button", { name: "设为默认" });
    await user.click(setDefault);
    await waitFor(() =>
      expect(rpc).toHaveBeenCalledWith("manual_set_codex_default_model", {
        modelId: "gpt-5.3-codex",
      }),
    );
  });
});
