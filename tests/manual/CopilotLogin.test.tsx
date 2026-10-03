import { StrictMode } from "react";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { CopilotLogin } from "@/manual/CopilotLogin";
import ManualApp from "@/manual/ManualApp";
import type { Snapshot } from "@/manual/types";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const rpc = vi.mocked(invoke);
const challenge = {
  flowId: "opaque-flow",
  userCode: "TEST-CODE",
  verificationUri: "https://github.com/login/device",
  expiresIn: 900,
  interval: 5,
};
const originalClipboard = Object.getOwnPropertyDescriptor(
  navigator,
  "clipboard",
);
let copy: ReturnType<typeof vi.fn>;
beforeEach(() => {
  rpc.mockReset();
  localStorage.clear();
  vi.useFakeTimers();
  copy = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: copy },
  });
  rpc.mockImplementation(async (command) =>
    command === "manual_copilot_start"
      ? challenge
      : command === "manual_copilot_poll"
        ? { state: "pending", interval: 5 }
        : undefined,
  );
});
afterEach(() => {
  cleanup();
  vi.clearAllTimers();
  vi.useRealTimers();
  vi.restoreAllMocks();
  if (originalClipboard)
    Object.defineProperty(navigator, "clipboard", originalClipboard);
  else Reflect.deleteProperty(navigator, "clipboard");
});
const click = async (element: HTMLElement) => {
  await act(async () => {
    fireEvent.click(element);
    await Promise.resolve();
  });
};
const tick = async (ms: number) => {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
};
const polls = () => rpc.mock.calls.filter(([c]) => c === "manual_copilot_poll");
const starts = () =>
  rpc.mock.calls.filter(([c]) => c === "manual_copilot_start");
describe("Copilot 短登录流程", () => {
  it("进入连接窗口就生成设备码，不需要二次点击，不自动打开浏览器", async () => {
    render(<CopilotLogin onClose={vi.fn()} onConnected={vi.fn()} />);
    await tick(0);
    expect(starts()).toHaveLength(1);
    expect(screen.getByLabelText("GitHub 设备授权码")).toHaveTextContent(
      "TEST-CODE",
    );
    expect(
      screen.queryByRole("button", { name: "登录并获取模型" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/私有目录/)).not.toBeInTheDocument();
    expect(rpc).not.toHaveBeenCalledWith("manual_copilot_open_login");
    await tick(4999);
    expect(polls()).toHaveLength(0);
    await tick(1);
    expect(polls()).toHaveLength(1);
    expect(polls()[0][1]).toEqual({ flowId: "opaque-flow" });
  });
  it("StrictMode 不会重复申请授权码", async () => {
    render(
      <StrictMode>
        <CopilotLogin onClose={vi.fn()} onConnected={vi.fn()} />
      </StrictMode>,
    );
    await tick(0);
    expect(starts()).toHaveLength(1);
    expect(screen.getByLabelText("GitHub 设备授权码")).toHaveTextContent(
      "TEST-CODE",
    );
  });
  it("一次点击复制设备码并通过后端打开固定 GitHub 地址", async () => {
    rpc.mockImplementation(async (command) =>
      command === "manual_copilot_start"
        ? { ...challenge, verificationUri: "https://untrusted.example" }
        : undefined,
    );
    render(<CopilotLogin onClose={vi.fn()} onConnected={vi.fn()} />);
    await tick(0);
    await click(screen.getByRole("button", { name: "复制代码并打开 GitHub" }));
    expect(copy).toHaveBeenCalledWith("TEST-CODE");
    expect(rpc).toHaveBeenCalledWith("manual_copilot_open_login");
    expect(screen.getByRole("status")).toHaveTextContent("代码已复制");
    expect(screen.queryByText(/untrusted.example/)).not.toBeInTheDocument();
  });
  it("复制失败仍打开浏览器，设备码可手动复制", async () => {
    copy.mockRejectedValue(new Error("clipboard unavailable"));
    render(<CopilotLogin onClose={vi.fn()} onConnected={vi.fn()} />);
    await tick(0);
    await click(screen.getByRole("button", { name: "复制代码并打开 GitHub" }));
    expect(rpc).toHaveBeenCalledWith("manual_copilot_open_login");
    expect(screen.getByRole("status")).toHaveTextContent("请手动复制上方代码");
  });
  it("浏览器打开失败时给出地址且允许重试，不重复生成设备码", async () => {
    rpc.mockImplementation(async (command) => {
      if (command === "manual_copilot_start") return challenge;
      if (command === "manual_copilot_open_login")
        throw new Error("cannot open");
    });
    render(<CopilotLogin onClose={vi.fn()} onConnected={vi.fn()} />);
    await tick(0);
    await click(screen.getByRole("button", { name: "复制代码并打开 GitHub" }));
    expect(screen.getByRole("status")).toHaveTextContent(
      "https://github.com/login/device",
    );
    expect(
      screen.getByRole("button", { name: "复制代码并打开 GitHub" }),
    ).not.toBeDisabled();
    expect(starts()).toHaveLength(1);
  });
  it("按 slow_down 返回间隔轮询，成功后停止所有计时器", async () => {
    const connected = vi.fn().mockResolvedValue(undefined);
    let n = 0;
    rpc.mockImplementation(async (c) =>
      c === "manual_copilot_start"
        ? challenge
        : c === "manual_copilot_poll"
          ? ++n === 1
            ? { state: "pending", interval: 10 }
            : {
                state: "connected",
                providerId: "copilot-personal",
                login: "fixture",
              }
          : undefined,
    );
    render(<CopilotLogin onClose={vi.fn()} onConnected={connected} />);
    await tick(0);
    await tick(5000);
    expect(polls()).toHaveLength(1);
    await tick(9999);
    expect(polls()).toHaveLength(1);
    await tick(1);
    expect(connected).toHaveBeenCalledWith("copilot-personal");
    await tick(20000);
    expect(polls()).toHaveLength(2);
  });
  it("取消等待先作废后端 flow，不再轮询", async () => {
    const close = vi.fn();
    render(<CopilotLogin onClose={close} onConnected={vi.fn()} />);
    await tick(0);
    await click(screen.getByRole("button", { name: "取消" }));
    expect(rpc).toHaveBeenCalledWith("manual_copilot_cancel", {
      flowId: "opaque-flow",
    });
    expect(close).toHaveBeenCalledOnce();
    await tick(30000);
    expect(polls()).toHaveLength(0);
  });
  it("生成代码期间关闭窗口，迟到的返回也会取消", async () => {
    let finish!: (value: typeof challenge) => void;
    rpc.mockImplementation((c) =>
      c === "manual_copilot_start"
        ? new Promise((resolve) => {
            finish = resolve;
          })
        : Promise.resolve(undefined),
    );
    const view = render(
      <CopilotLogin onClose={vi.fn()} onConnected={vi.fn()} />,
    );
    await tick(0);
    view.unmount();
    await act(async () => {
      finish(challenge);
    });
    expect(rpc).toHaveBeenCalledWith("manual_copilot_cancel", {
      flowId: "opaque-flow",
    });
    expect(polls()).toHaveLength(0);
  });
  it("过期后停止轮询并提供重新生成入口", async () => {
    rpc.mockImplementation(async (c) =>
      c === "manual_copilot_start" ? { ...challenge, expiresIn: 3 } : undefined,
    );
    render(<CopilotLogin onClose={vi.fn()} onConnected={vi.fn()} />);
    await tick(0);
    await tick(3000);
    expect(screen.getByRole("alert")).toHaveTextContent("授权码已过期");
    expect(
      screen.getByRole("button", { name: "重新生成授权码" }),
    ).toBeVisible();
    expect(polls()).toHaveLength(0);
    await click(screen.getByRole("button", { name: "重新生成授权码" }));
    expect(starts()).toHaveLength(2);
  });
  it("拒绝授权或无权限时显示错误，不触发模型调用", async () => {
    const connected = vi.fn();
    rpc.mockImplementation(async (c) => {
      if (c === "manual_copilot_start") return challenge;
      if (c === "manual_copilot_poll") throw "Copilot 权限不足，请检查订阅";
    });
    render(<CopilotLogin onClose={vi.fn()} onConnected={connected} />);
    await tick(0);
    await tick(5000);
    expect(screen.getByRole("alert")).toHaveTextContent("Copilot 权限不足");
    expect(connected).not.toHaveBeenCalled();
  });
  it("应用启动不授权，点击入口才生成码；授权成功后只拉目录", async () => {
    const snapshot: Snapshot = {
      document: {
        revision: 0,
        providers: [
          {
            id: "original",
            name: "Existing API",
            baseUrl: "https://existing.example/v1",
            keyEnv: "",
            protocol: "openai_chat",
            enabled: true,
            models: [],
          },
        ],
        policies: {},
        tests: {},
      },
      groups: [],
      status: { running: false },
      baseUrl: "http://127.0.0.1:15722",
    };
    const original = structuredClone(snapshot.document.providers[0]);
    rpc.mockImplementation(async (c) => {
      if (c === "manual_snapshot") return structuredClone(snapshot);
      if (c === "manual_copilot_start") return challenge;
      if (c === "manual_copilot_poll") {
        snapshot.document.providers.push({
          id: "copilot-personal",
          name: "GitHub Copilot",
          baseUrl: "https://api.githubcopilot.com",
          keyEnv: "",
          copilot: { accountId: "123", grantId: "fixture-grant" },
          protocol: "openai_chat",
          models: [],
          enabled: true,
        });
        snapshot.document.revision++;
        snapshot.copilotAuth = {
          connected: true,
          login: "fixture-user",
          message: null,
        };
        return { state: "connected", providerId: "copilot-personal" };
      }
      if (c === "manual_discover") {
        snapshot.document.providers[1].models = [
          {
            id: "copilot/fixture-model",
            protocolOverride: "openai_chat",
            image: false,
            contextWindow: 128000,
            tools: true,
            enabled: true,
          },
        ];
        return 1;
      }
    });
    render(<ManualApp initialWorkspace="providers" />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(starts()).toHaveLength(0);
    await click(screen.getByRole("button", { name: "GitHub Copilot" }));
    await tick(0);
    expect(starts()).toHaveLength(1);
    await tick(5000);
    expect(
      screen.queryByRole("dialog", { name: "连接 GitHub Copilot" }),
    ).not.toBeInTheDocument();
    expect(
      within(
        screen.getByRole("region", { name: "来源 GitHub Copilot" }),
      ).getByText(/fixture-user/),
    ).toBeVisible();
    expect(snapshot.document.providers[0]).toEqual(original);
    const commands = rpc.mock.calls.map(([c]) => c);
    expect(commands.filter((c) => c === "manual_discover")).toHaveLength(1);
    for (const forbidden of [
      "manual_test",
      "manual_save",
      "manual_gateway",
      "manual_launch",
      "manual_apply_sync",
      "manual_copilot_open_login",
    ])
      expect(commands).not.toContain(forbidden);
  });
});
