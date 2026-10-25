import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { Updates, UpdateBanner, type UpdateView } from "@/manual/Updates";
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  isTauri: () => true,
}));
let notify: (event: { payload: UpdateView }) => void;
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_: string, handler: typeof notify) => {
    notify = handler;
    return () => {};
  }),
}));
let view: UpdateView;
const rpc = vi.mocked(invoke);
beforeEach(() => {
  view = {
    phase: "available",
    currentVersion: "3.24.7",
    version: "3.24.8",
    notes: "官方更新说明",
    autoCheck: true,
    prompt: true,
    received: 0,
    total: null,
    active: 0,
    message: "发现新版本",
    checkedAt: null,
  };
  rpc.mockReset();
  rpc.mockImplementation(async (name) => {
    if (name === "manual_update_state") return view;
    if (name === "manual_update_later") {
      view = { ...view, prompt: false };
      notify({ payload: view });
      return;
    }
    throw new Error(`unexpected command ${name}`);
  });
});
it("shows a non-blocking banner and dismiss removes it without downloading", async () => {
  const onInstall = vi.fn();
  const onDismiss = vi.fn();
  render(
    <UpdateBanner view={view} onInstall={onInstall} onDismiss={onDismiss} />,
  );
  expect(screen.getByText("发现新版本 3.24.8")).toBeVisible();
  expect(rpc).not.toHaveBeenCalledWith(
    "manual_update_install",
    expect.anything(),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "关闭更新提示" }),
  );
  expect(onDismiss).toHaveBeenCalled();
  expect(onInstall).not.toHaveBeenCalled();
});
it("banner install button triggers install callback", async () => {
  const onInstall = vi.fn();
  render(
    <UpdateBanner view={view} onInstall={onInstall} onDismiss={vi.fn()} />,
  );
  await userEvent.click(screen.getByRole("button", { name: "下载并更新" }));
  expect(onInstall).toHaveBeenCalled();
});
it("keeps cancellation available while the native request waits for a stream", async () => {
  let finish: (() => void) | undefined;
  rpc.mockImplementation(async (name) => {
    if (name === "manual_update_state") return view;
    if (name === "manual_update_install") {
      view = {
        ...view,
        phase: "waiting",
        active: 1,
        message: "等待 1 个请求结束",
      };
      notify({ payload: view });
      return new Promise<void>((resolve) => {
        finish = resolve;
      });
    }
    if (name === "manual_update_later") {
      view = { ...view, phase: "ready", prompt: false, message: "已恢复服务" };
      notify({ payload: view });
      finish?.();
      return;
    }
    throw new Error(`unexpected command ${name}`);
  });
  render(<Updates settingsVisible />);
  await screen.findByText("当前版本 3.24.7");
  await userEvent.click(
    screen.getByRole("button", { name: /下载并更新/ }),
  );
  expect(await screen.findByText("等待 1 个请求结束")).toBeVisible();
  await userEvent.click(
    screen.getByRole("button", { name: /取消更新|稍后/ }),
  );
  expect(await screen.findByText("已恢复服务")).toBeVisible();
});
it("shows check errors inline without opening unsolicited dialogs", async () => {
  view = { ...view, prompt: false, version: null, phase: "idle" };
  render(<Updates settingsVisible />);
  await screen.findByText("当前版本 3.24.7");
  act(() => {
    view = { ...view, phase: "error", message: "检查失败，网关不受影响" };
    notify({ payload: view });
  });
  expect(screen.getByText("检查失败，网关不受影响")).toBeVisible();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "检查更新" })).toBeEnabled();
});
