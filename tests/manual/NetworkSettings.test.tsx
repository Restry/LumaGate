import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import ManualApp from "@/manual/ManualApp";
import type {
  AccessKeyOperation,
  GatewayNetworkSettings,
  GatewayNetworkUpdate,
  Snapshot,
} from "@/manual/types";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const rpc = vi.mocked(invoke);
const A = "fixture-key-a-0123456789abcdef0123456789abcdef";
const B = "fixture-key-b-0123456789abcdef0123456789abcdef";
let snapshot: Snapshot;
let secrets: Map<string, string>;
const local: GatewayNetworkSettings = {
  revision: 0,
  listenAddress: "127.0.0.1",
  listenPort: 15722,
  allowLan: false,
};
function commands() {
  return rpc.mock.calls.map(([command]) => command);
}
async function open() {
  const user = userEvent.setup();
  const view = render(<ManualApp />);
  await user.click(await screen.findByRole("button", { name: "设置" }));
  return { user, view };
}
async function allowLan(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("combobox", { name: "监听地址" }));
  await user.click(
    await screen.findByRole("option", { name: "0.0.0.0 · 允许内网访问" }),
  );
}
function seed(id: string, name: string, value: string, canCopy = true) {
  snapshot.network!.accessKeys.items.push({
    id,
    name,
    enabled: true,
    canCopy,
    createdAt: null,
  });
  secrets.set(id, value);
}
async function add(
  user: ReturnType<typeof userEvent.setup>,
  name: string,
  value = A,
) {
  await user.click(screen.getByRole("button", { name: "添加密钥" }));
  const dialog = screen.getByRole("dialog", { name: "添加访问密钥" });
  await user.type(within(dialog).getByLabelText("密钥名称"), name);
  await user.type(within(dialog).getByLabelText("密钥内容"), value);
  await user.click(within(dialog).getByRole("button", { name: "保存密钥" }));
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
}
beforeEach(() => {
  secrets = new Map();
  snapshot = {
    document: { revision: 0, providers: [], policies: {}, tests: {} },
    groups: [],
    status: { running: true },
    baseUrl: "http://127.0.0.1:15722",
    network: {
      saved: { ...local },
      active: { ...local },
      restartRequired: false,
      lanAddresses: ["http://192.168.1.10:15722/v1"],
      accessKeys: { revision: 0, items: [] },
    },
  };
  rpc.mockReset();
  rpc.mockImplementation(async (command, raw) => {
    if (command === "manual_snapshot") return structuredClone(snapshot);
    if (
      command === "manual_reveal_access_key" ||
      command === "manual_copy_access_key"
    ) {
      const { keyId } = raw as { keyId: string };
      if (
        !snapshot.network!.accessKeys.items.find((key) => key.id === keyId)
          ?.canCopy
      )
        throw new Error("请补录一次原密钥以启用复制");
      if (command === "manual_copy_access_key") {
        await navigator.clipboard.writeText(secrets.get(keyId)!);
        return;
      }
      return secrets.get(keyId)!;
    }
    if (command === "manual_update_access_key") {
      const { revision, operation } = raw as {
        revision: number;
        operation: AccessKeyOperation;
      };
      const keys = snapshot.network!.accessKeys;
      if (revision !== keys.revision)
        throw new Error("访问密钥列表已变化，请刷新后重试");
      if (operation.action === "add") {
        const id = `key-${keys.revision}`;
        seed(id, operation.name, operation.value);
      } else {
        const key = keys.items.find((key) => key.id === operation.keyId)!;
        if (operation.action === "rename") key.name = operation.name;
        if (operation.action === "set_enabled") key.enabled = operation.enabled;
        if (operation.action === "remove") {
          keys.items = keys.items.filter((item) => item.id !== key.id);
          secrets.delete(key.id);
        }
        if (operation.action === "restore_copy") {
          if (secrets.get(key.id) !== operation.value)
            throw new Error("与原密钥不匹配；未修改原密钥或访问权限");
          key.canCopy = true;
        }
      }
      keys.revision += 1;
      return { keys: structuredClone(keys), warnings: [] };
    }
    if (command === "manual_save_gateway_settings") {
      const { settings } = raw as { settings: GatewayNetworkUpdate };
      snapshot.network!.saved = {
        ...settings,
        revision: settings.revision + 1,
        allowLan: settings.listenAddress === "0.0.0.0",
      };
      snapshot.network!.restartRequired = true;
      return structuredClone(snapshot.network!.saved);
    }
    if (command === "manual_request_logs") return [];
    throw new Error(`Unexpected mutation: ${command}`);
  });
});

describe("多密钥与网络设置", () => {
  it("打开设置只读元数据，不读取密钥明文或启动网关", async () => {
    seed("saved", "MacBook", A);
    await open();
    expect(
      screen.getByRole("button", { name: "复制密钥 MacBook" }),
    ).toBeEnabled();
    expect(screen.getByLabelText("监听端口")).toHaveValue("15722");
    expect(commands()).toEqual(["manual_snapshot"]);
    expect(document.body.textContent).not.toContain(A);
  });
  it("添加后可反复复制，重新打开界面仍可复制，快照不含明文", async () => {
    const { user, view } = await open();
    const clipboard = vi
      .spyOn(navigator.clipboard, "writeText")
      .mockResolvedValue();
    await add(user, "MacBook");
    expect(rpc).toHaveBeenCalledWith("manual_update_access_key", {
      revision: 0,
      operation: { action: "add", name: "MacBook", value: A },
    });
    await user.click(screen.getByRole("button", { name: "复制密钥 MacBook" }));
    await waitFor(() => expect(clipboard).toHaveBeenCalledWith(A));
    await user.click(screen.getByRole("button", { name: "复制密钥 MacBook" }));
    await waitFor(() => expect(clipboard).toHaveBeenCalledTimes(2));
    view.unmount();
    render(<ManualApp />);
    await user.click(await screen.findByRole("button", { name: "设置" }));
    await user.click(screen.getByRole("button", { name: "复制密钥 MacBook" }));
    await waitFor(() => expect(clipboard).toHaveBeenCalledTimes(3));
    expect(JSON.stringify(snapshot)).not.toContain(A);
    expect(commands()).not.toContain("manual_reveal_access_key");
    expect(commands()).not.toContain("manual_gateway");
    expect(commands()).not.toContain("manual_apply_sync");
  });
  it("新增第二把不替换第一把，停用立即保存，删除需要明确确认", async () => {
    seed("a", "Laptop", A);
    const { user } = await open();
    await add(user, "CI", B);
    expect(
      screen.getByRole("button", { name: "复制密钥 Laptop" }),
    ).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "停用密钥 Laptop" }));
    expect(
      await screen.findByRole("button", { name: "启用密钥 Laptop" }),
    ).toBeVisible();
    const writes = commands().filter(
      (command) => command === "manual_update_access_key",
    ).length;
    await user.click(screen.getByRole("button", { name: "删除密钥 CI" }));
    expect(
      commands().filter((command) => command === "manual_update_access_key"),
    ).toHaveLength(writes);
    await user.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "确认删除",
      }),
    );
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "复制密钥 CI" }),
      ).not.toBeInTheDocument(),
    );
    expect(snapshot.network!.accessKeys.items).toHaveLength(1);
    expect(commands()).not.toContain("manual_gateway");
  });
  it("旧版密钥匹配补录一次后可复制；错误补录保留输入和原权限", async () => {
    seed("legacy", "旧版密钥", A, false);
    const { user } = await open();
    expect(
      screen.getByRole("button", { name: "复制密钥 旧版密钥" }),
    ).toBeDisabled();
    await user.click(
      screen.getByRole("button", { name: "补录原密钥 旧版密钥" }),
    );
    const dialog = screen.getByRole("dialog");
    const input = within(dialog).getByLabelText("原密钥");
    await user.type(input, B);
    await user.click(within(dialog).getByRole("button", { name: "保存密钥" }));
    expect(await within(dialog).findByText(/与原密钥不匹配/)).toBeVisible();
    expect(input).toHaveValue(B);
    expect(secrets.get("legacy")).toBe(A);
    await user.clear(input);
    await user.type(input, A);
    await user.click(within(dialog).getByRole("button", { name: "保存密钥" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(
      screen.getByRole("button", { name: "复制密钥 旧版密钥" }),
    ).toBeEnabled();
    expect(snapshot.network!.accessKeys.items[0].enabled).toBe(true);
  });
  it("剪贴板被拒绝时提供手动复制，不要求重建密钥", async () => {
    seed("a", "MacBook", A);
    const { user } = await open();
    vi.spyOn(navigator.clipboard, "writeText").mockRejectedValue(
      new Error("denied"),
    );
    await user.click(screen.getByRole("button", { name: "复制密钥 MacBook" }));
    const dialog = await screen.findByRole("dialog", {
      name: "手动复制访问密钥",
    });
    expect(within(dialog).getByLabelText("要复制的访问密钥")).toHaveValue(A);
    await user.click(within(dialog).getByRole("button", { name: "完成" }));
    expect(screen.queryByLabelText("要复制的访问密钥")).not.toBeInTheDocument();
    expect(commands()).toEqual([
      "manual_snapshot",
      "manual_copy_access_key",
      "manual_reveal_access_key",
    ]);
  });
  it("密钥输入校验保留可重试操作并聚焦错误字段", async () => {
    const { user } = await open();
    await user.click(screen.getByRole("button", { name: "添加密钥" }));
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "保存密钥" }));
    expect(within(dialog).getByLabelText("密钥名称")).toHaveFocus();
    await user.type(within(dialog).getByLabelText("密钥名称"), "Notebook");
    await user.click(within(dialog).getByRole("button", { name: "保存密钥" }));
    expect(within(dialog).getByLabelText("密钥内容")).toHaveFocus();
    await user.click(
      within(dialog).getByRole("button", { name: "生成随机密钥" }),
    );
    const generated =
      within(dialog).getByLabelText<HTMLInputElement>("密钥内容");
    expect(generated.value).toMatch(/^ccm_[a-f0-9]{64}$/);
    expect(commands()).toEqual(["manual_snapshot"]);
  });
  it("端口无效或没有启用密钥时，不发送监听配置写入", async () => {
    const { user } = await open();
    const port = screen.getByLabelText("监听端口");
    await user.clear(port);
    await user.type(port, "65536");
    await user.click(screen.getByRole("button", { name: "保存网络设置" }));
    expect(port).toHaveFocus();
    expect(port).toHaveAttribute("aria-invalid", "true");
    await user.clear(port);
    await user.type(port, "18080");
    await allowLan(user);
    await user.click(screen.getByRole("button", { name: "保存网络设置" }));
    expect(screen.getByRole("button", { name: "添加密钥" })).toHaveFocus();
    expect(
      screen.getByText("请先添加并启用至少一把访问密钥，再允许内网访问。"),
    ).toBeVisible();
    expect(commands()).toEqual(["manual_snapshot"]);
  });
  it("新增密钥不覆盖未保存的监听草稿，也不使监听版本失效", async () => {
    const { user } = await open();
    const port = screen.getByLabelText("监听端口");
    await user.clear(port);
    await user.type(port, "18080");
    await allowLan(user);
    await add(user, "Laptop");
    expect(port).toHaveValue("18080");
    await user.click(screen.getByRole("button", { name: "模型" }));
    await user.click(screen.getByRole("button", { name: "设置" }));
    await user.click(screen.getByRole("button", { name: "保存网络设置" }));
    await waitFor(() =>
      expect(rpc).toHaveBeenCalledWith("manual_save_gateway_settings", {
        settings: { revision: 0, listenAddress: "0.0.0.0", listenPort: 18080 },
      }),
    );
    expect(await screen.findByText("已保存，等待手动重启网关")).toBeVisible();
    expect(
      within(
        screen.getByRole("complementary", { name: "当前生效的连接" }),
      ).getByText("127.0.0.1:15722"),
    ).toBeVisible();
    expect(commands()).not.toContain("manual_gateway");
  });
  it("重命名使用稳定 keyId，不需要输入或重写密钥", async () => {
    seed("a", "Old", A);
    const { user } = await open();
    await user.click(screen.getByRole("button", { name: "重命名密钥 Old" }));
    const dialog = screen.getByRole("dialog");
    await user.clear(within(dialog).getByLabelText("密钥名称"));
    await user.type(within(dialog).getByLabelText("密钥名称"), "New");
    await user.click(within(dialog).getByRole("button", { name: "保存密钥" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(rpc).toHaveBeenCalledWith("manual_update_access_key", {
      revision: 0,
      operation: { action: "rename", keyId: "a", name: "New" },
    });
    expect(secrets.get("a")).toBe(A);
  });
});
