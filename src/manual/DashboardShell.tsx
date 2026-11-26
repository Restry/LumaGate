import { useEffect, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Check,
  CircleHelp,
  Copy,
  FileClock,
  Layers3,
  LayoutDashboard,
  Maximize2,
  Minimize2,
  Minus,
  Moon,
  Play,
  Settings2,
  Square,
  SquareTerminal,
  Sun,
  X,
} from "lucide-react";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { Snapshot } from "./types";
import type { Workspace } from "./workspaces";
import { version as appVersion } from "../../package.json";
import brandMark from "./assets/lumagate-mark.svg";

const primaryNav = [
  { id: "overview", name: "首页", label: "概览", icon: LayoutDashboard },
  { id: "sources", name: "Providers", label: "来源与模型", icon: Layers3 },
  { id: "agents", name: "Agents", label: "客户端", icon: SquareTerminal },
  { id: "logs", name: "日志", label: "调用日志", icon: FileClock },
] as const;

const utilityNav = [
  { id: "settings", name: "设置", label: "设置", icon: Settings2 },
  { id: "help", name: "帮助", label: "帮助", icon: CircleHelp },
] as const;

const allNav = [...primaryNav, ...utilityNav] as const;

function countConnectedAgents(snapshot: Snapshot | null): number {
  const a = snapshot?.agents;
  if (!a) return 0;
  return [
    a.codex?.connected,
    a.claude?.connected,
    a.pi?.connected,
    a.opencode?.connected,
    a.openclaw?.connected,
    a.hermes?.connected,
    a.droid?.connected,
  ].filter(Boolean).length;
}

export function DashboardShell({
  snapshot,
  tab,
  onTab,
  busy,
  onGateway,
  onCopyDirectory,
  hasUpdate = false,
  children,
}: {
  snapshot: Snapshot | null;
  tab: Workspace;
  onTab: (tab: Workspace) => void;
  busy: boolean;
  onGateway: () => void;
  onCopyDirectory: () => void;
  hasUpdate?: boolean;
  children: ReactNode;
}) {
  const [copiedAddr, setCopiedAddr] = useState(false);
  const [maximized, setMaximized] = useState(false);
  const [dark, setDark] = useState(() => {
    try {
      const saved = localStorage.getItem("manual.appearance");
      return saved
        ? saved === "dark"
        : document.documentElement.classList.contains("dark");
    } catch {
      return document.documentElement.classList.contains("dark");
    }
  });

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    try {
      localStorage.setItem("manual.appearance", dark ? "dark" : "light");
    } catch {
      /* Appearance persistence is optional */
    }
  }, [dark]);

  useEffect(() => {
    let disposed = false;
    try {
      const win = getCurrentWindow();
      void win
        .isMaximized()
        .then((v) => {
          if (!disposed) setMaximized(v);
        })
        .catch(() => undefined);
    } catch {
      /* Browser / test environment without Tauri window */
    }
    return () => {
      disposed = true;
    };
  }, []);

  const handleMinimize = () => {
    try {
      void getCurrentWindow().minimize();
    } catch {
      /* noop outside Tauri */
    }
  };
  const handleToggleMaximize = async () => {
    try {
      const win = getCurrentWindow();
      await win.toggleMaximize();
      setMaximized(await win.isMaximized());
    } catch {
      /* noop outside Tauri */
    }
  };
  const handleClose = () => {
    try {
      void getCurrentWindow().close();
    } catch {
      /* noop outside Tauri */
    }
  };
  const handleTitlebarMouseDown = (
    event: React.MouseEvent<HTMLElement>,
  ) => {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement | null;
    if (
      target?.closest("button, a, input, select, textarea, [role='button']")
    ) {
      return;
    }
    if (event.detail === 2) {
      void handleToggleMaximize();
      return;
    }
    if (target?.hasAttribute("data-tauri-drag-region")) {
      return;
    }
    try {
      void getCurrentWindow().startDragging();
    } catch {
      /* noop outside Tauri */
    }
  };

  const running = !!snapshot?.status.running;
  const endpointLabel = snapshot?.baseUrl
    ? snapshot.baseUrl.replace(/^https?:\/\//, "")
    : "127.0.0.1:15722";
  const connectedAgents = countConnectedAgents(snapshot);
  const activeProviders =
    snapshot?.document.providers.filter((p) => p.enabled).length ?? 0;
  const isWindows =
    typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);

  const copyAddr = () => {
    onCopyDirectory();
    setCopiedAddr(true);
    setTimeout(() => setCopiedAddr(false), 1500);
  };

  return (
    <TooltipProvider delayDuration={250}>
      <div
        className={`mg-app mg-dashboard-client mg-topchrome-shell ${isWindows ? "is-windows" : ""} ${maximized ? "is-maximized" : ""}`}
      >
        <a className="mg-skip-link" href="#manual-content">
          跳到工作区内容
        </a>

        {/* Custom frameless top titlebar & navigation bar */}
        <header
          className="mg-titlebar"
          data-tauri-drag-region
          onMouseDown={handleTitlebarMouseDown}
        >
          <div className="mg-titlebar-left" data-tauri-drag-region>
            {!isWindows && (
              <div className="mg-win-controls" aria-label="窗口控制">
                <button
                  type="button"
                  className="mg-win-btn is-close"
                  aria-label="关闭窗口"
                  title="关闭"
                  onClick={handleClose}
                >
                  <X size={8} strokeWidth={2.6} aria-hidden />
                </button>
                <button
                  type="button"
                  className="mg-win-btn is-min"
                  aria-label="最小化窗口"
                  title="最小化"
                  onClick={handleMinimize}
                >
                  <Minus size={8} strokeWidth={2.6} aria-hidden />
                </button>
                <button
                  type="button"
                  className="mg-win-btn is-max"
                  aria-label={maximized ? "还原窗口" : "最大化窗口"}
                  title={maximized ? "还原" : "最大化"}
                  onClick={() => void handleToggleMaximize()}
                >
                  {maximized ? (
                    <Minimize2 size={7} strokeWidth={2.4} aria-hidden />
                  ) : (
                    <Maximize2 size={7} strokeWidth={2.4} aria-hidden />
                  )}
                </button>
              </div>
            )}

            <button
              className="mg-brand"
              type="button"
              aria-label="LumaGate 首页"
              title={`LumaGate v${appVersion}`}
              onClick={() => onTab("overview")}
            >
              <span className="mg-brand-mark">
                <img src={brandMark} alt="" aria-hidden />
              </span>
              <strong>LumaGate</strong>
            </button>
          </div>

          <nav className="mg-topnav" aria-label="主导航">
            {primaryNav.map((entry) => {
              const badge =
                entry.id === "sources" && activeProviders > 0
                  ? activeProviders
                  : entry.id === "agents" && connectedAgents > 0
                    ? connectedAgents
                    : null;
              return (
                <button
                  key={entry.id}
                  type="button"
                  className="mg-topnav-item"
                  aria-label={entry.label}
                  aria-current={tab === entry.id ? "page" : undefined}
                  onClick={() => onTab(entry.id)}
                >
                  <entry.icon size={14} strokeWidth={1.9} aria-hidden />
                  <span>{entry.name}</span>
                  {badge !== null && (
                    <em className="mg-topnav-count">{badge}</em>
                  )}
                </button>
              );
            })}
          </nav>

          <div className="mg-titlebar-right" data-tauri-drag-region>
            <div className="mg-gateway-pill">
              <button
                type="button"
                className={`mg-gateway-toggle ${running ? "is-running" : ""}`}
                disabled={
                  busy ||
                  !snapshot ||
                  (!running && snapshot.logStorage?.ready === false)
                }
                aria-label={running ? "停止" : "启动网关"}
                title={running ? "点击停止网关" : "点击启动本地网关"}
                onClick={onGateway}
              >
                <i aria-hidden />
                <span>{!snapshot ? "连接中" : running ? "运行中" : "启动网关"}</span>
                {running ? <Square size={9} aria-hidden /> : <Play size={9} aria-hidden />}
              </button>
              <button
                type="button"
                className="mg-endpoint-copy"
                disabled={!snapshot}
                onClick={copyAddr}
                title={`复制模型目录地址 (${endpointLabel})`}
                aria-label="复制目录地址"
              >
                <code>{endpointLabel}</code>
                {copiedAddr ? <Check size={11} /> : <Copy size={11} />}
              </button>
            </div>

            <div className="mg-titlebar-utils" aria-label="辅助导航">
              {utilityNav.map((entry) => (
                <Tooltip key={entry.id}>
                  <TooltipTrigger asChild>
                    <button
                      type="button"
                      className={`mg-util-btn ${tab === entry.id ? "is-active" : ""}`}
                      aria-label={entry.label}
                      aria-current={tab === entry.id ? "page" : undefined}
                      onClick={() => onTab(entry.id)}
                    >
                      <entry.icon size={15} strokeWidth={1.85} aria-hidden />
                      {entry.id === "settings" && hasUpdate && (
                        <i className="mg-nav-badge" aria-label="有可用更新" />
                      )}
                    </button>
                  </TooltipTrigger>
                  <TooltipContent side="bottom">{entry.name}</TooltipContent>
                </Tooltip>
              ))}
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    type="button"
                    className="mg-util-btn"
                    aria-label="切换外观"
                    onClick={() => setDark((v) => !v)}
                  >
                    {dark ? <Sun size={15} /> : <Moon size={15} />}
                  </button>
                </TooltipTrigger>
                <TooltipContent side="bottom">
                  {dark ? "浅色外观" : "深色外观"}
                </TooltipContent>
              </Tooltip>
            </div>

            {isWindows && (
              <div className="mg-win-caption-controls" aria-label="窗口控制">
                <button
                  type="button"
                  className="mg-win-caption-btn"
                  aria-label="最小化窗口"
                  title="最小化"
                  onClick={handleMinimize}
                >
                  <Minus size={13} strokeWidth={1.8} aria-hidden />
                </button>
                <button
                  type="button"
                  className="mg-win-caption-btn"
                  aria-label={maximized ? "还原窗口" : "最大化窗口"}
                  title={maximized ? "还原" : "最大化"}
                  onClick={() => void handleToggleMaximize()}
                >
                  {maximized ? (
                    <Minimize2 size={12} strokeWidth={1.8} aria-hidden />
                  ) : (
                    <Maximize2 size={12} strokeWidth={1.8} aria-hidden />
                  )}
                </button>
                <button
                  type="button"
                  className="mg-win-caption-btn is-close"
                  aria-label="关闭窗口"
                  title="关闭"
                  onClick={handleClose}
                >
                  <X size={14} strokeWidth={1.8} aria-hidden />
                </button>
              </div>
            )}
          </div>
        </header>

        <main
          className="mg-content mg-stage-content"
          id="manual-content"
          tabIndex={-1}
          data-page={allNav.find((n) => n.id === tab)?.id ?? tab}
        >
          {children}
        </main>
      </div>
    </TooltipProvider>
  );
}
