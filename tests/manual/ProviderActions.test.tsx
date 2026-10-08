import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
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
      providers: [
        {
          id: "copilot-personal",
          name: "GitHub Copilot",
          baseUrl: "https://api.githubcopilot.com",
          keyEnv: "",
          copilot: { accountId: "123", grantId: "fixture" },
          protocol: "openai_chat",
          enabled: true,
          models: [
            {
              id: "copilot/example",
              enabled: false,
              image: false,
              tools: null,
              contextWindow: null,
            },
          ],
        },
      ],
      policies: {},
      tests: {},
    },
    groups: [],
    status: { running: false },
    baseUrl: "http://127.0.0.1:15722",
    copilotAuth: { connected: true, login: "fixture-user", message: null },
  };
  rpc.mockReset();
  rpc.mockImplementation(async (command, args) => {
    if (command === "manual_snapshot") return structuredClone(data);
    if (command === "manual_set_provider_enabled") {
      const update = args as { providerId: string; enabled: boolean };
      data.document.providers.find((p) => p.id === update.providerId)!.enabled =
        update.enabled;
      data.document.revision++;
    }
    if (command === "manual_set_provider_cost_estimation") {
      const update = args as { providerId: string; enabled: boolean };
      data.document.providers.find(
        (p) => p.id === update.providerId,
      )!.costEstimationEnabled = update.enabled;
      data.document.revision++;
    }
  });
});
it("已连接来源在模型页可刷新，等待时防重入，失败保留目录并可重试", async () => {
  const user = userEvent.setup();
  let reject!: (reason: Error) => void;
  rpc.mockImplementation(async (command) => {
    if (command === "manual_snapshot") return structuredClone(data);
    if (command === "manual_discover")
      return new Promise((_, fail) => {
        reject = fail;
      });
  });
  render(<ManualApp initialWorkspace="providers" />);
  const region = await screen.findByRole("region", {
    name: "来源 GitHub Copilot",
  });
  await user.click(within(region).getByRole("button", { name: /来源模型/ }));
  const refresh = within(region).getByRole("button", { name: "刷新模型" });
  await user.dblClick(refresh);
  expect(
    within(region).getByRole("button", { name: "正在刷新模型…" }),
  ).toBeDisabled();
  expect(rpc.mock.calls.filter(([c]) => c === "manual_discover")).toHaveLength(
    1,
  );
  await act(async () =>
    reject(new Error("GitHub/Copilot 授权失效，请重新登录")),
  );
  expect(within(region).getByRole("alert")).toHaveTextContent(
    "已有目录和启停选择保持不变",
  );
  expect(within(region).getByText("copilot/example")).toBeVisible();
  expect(
    within(region).getByRole("button", { name: "刷新模型" }),
  ).toBeEnabled();
  expect(
    within(region).getByRole("button", { name: "重新登录 GitHub Copilot" }),
  ).toBeVisible();
  rpc.mockImplementation(async (command) =>
    command === "manual_snapshot" ? structuredClone(data) : 1,
  );
  await user.click(within(region).getByRole("button", { name: "刷新模型" }));
  await waitFor(() =>
    expect(within(region).getByRole("status")).toHaveTextContent(
      "已获取 1 个模型",
    ),
  );
  expect(data.document.providers[0].models[0].enabled).toBe(false);
  expect(
    rpc.mock.calls.some(([c]) =>
      ["manual_test", "manual_apply_sync", "manual_copilot_start"].includes(c),
    ),
  ).toBe(false);
});
it("来源标题直接停用并恢复，保存模型和账号绑定，不打开编辑弹窗", async () => {
  const user = userEvent.setup();
  const original = structuredClone(data.document.providers[0]);
  render(<ManualApp initialWorkspace="providers" />);
  const toggle = await screen.findByRole("switch", {
    name: "启用来源 GitHub Copilot",
  });
  await user.click(toggle);
  await waitFor(() => expect(toggle).not.toBeChecked());
  expect(data.document.providers[0]).toEqual({ ...original, enabled: false });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  await user.click(toggle);
  await waitFor(() => expect(toggle).toBeChecked());
  expect(data.document.providers[0]).toEqual(original);
});

it("Copilot 的费用开关默认开启，停用路由后仍可独立保存且重载保留", async () => {
  const user = userEvent.setup();
  const original = structuredClone(data.document.providers[0]);
  data.document.providers[0].enabled = false;
  const view = render(<ManualApp initialWorkspace="providers" />);
  const toggle = await screen.findByRole("switch", { name: "参与费用估算" });
  expect(toggle).toBeChecked();
  await user.click(toggle);
  await waitFor(() => expect(toggle).not.toBeChecked());
  expect(data.document.providers[0]).toEqual({
    ...original,
    enabled: false,
    costEstimationEnabled: false,
  });
  view.unmount();
  render(<ManualApp initialWorkspace="providers" />);
  const restored = await screen.findByRole("switch", { name: "参与费用估算" });
  expect(restored).not.toBeChecked();
  await user.click(restored);
  await waitFor(() => expect(restored).toBeChecked());
  expect(
    screen.getByRole("switch", { name: "启用来源 GitHub Copilot" }),
  ).not.toBeChecked();
});

it("API Provider 新建默认计价，编辑关闭后显式保存", async () => {
  const user = userEvent.setup();
  const { ProviderEditor } = await import("@/manual/dialogs");
  const { emptySource } = await import("@/manual/types");
  const source = {
    ...emptySource(),
    name: "API fixture",
    baseUrl: "https://api.example/v1",
  };
  const save = vi.fn();
  render(
    <ProviderEditor
      source={source}
      isNew
      busy={false}
      onClose={() => {}}
      onSave={save}
    />,
  );
  const toggle = screen.getByRole("switch", { name: "参与费用估算" });
  expect(toggle).toBeChecked();
  await user.click(toggle);
  expect(save).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "保存，不同步" }));
  expect(save.mock.calls[0][0]).toEqual({
    ...source,
    costEstimationEnabled: false,
  });
});
