import { useEffect, useState, type ReactNode } from "react";
import {
  Box,
  CircleHelp,
  Copy,
  FileClock,
  Layers3,
  LayoutDashboard,
  Moon,
  PanelLeftClose,
  PanelLeftOpen,
  Play,
  Settings2,
  Square,
  SquareTerminal,
  Sun,
} from "lucide-react";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { ActionButton } from "./ui";
import type { Snapshot } from "./types";
import type { Workspace } from "./workspaces";
import { version as appVersion } from "../../package.json";
import brandMark from "./assets/lumagate-mark.svg";

const navigation = [
  { id: "overview", name: "概览", label: "概览", icon: LayoutDashboard },
  { id: "logs", name: "日志", label: "调用日志", icon: FileClock },
  { id: "providers", name: "Provider", label: "Provider", icon: Layers3 },
  { id: "models", name: "模型", label: "模型", icon: Box },
  { id: "agents", name: "客户端", label: "客户端", icon: SquareTerminal },
  { id: "settings", name: "设置", label: "设置", icon: Settings2 },
  { id: "help", name: "帮助", label: "帮助", icon: CircleHelp },
] as const;

export function DashboardShell({
  snapshot,
  tab,
  onTab,
  busy,
  onGateway,
  onCopyDirectory,
  children,
}: {
  snapshot: Snapshot | null;
  tab: Workspace;
  onTab: (tab: Workspace) => void;
  busy: boolean;
  onGateway: () => void;
  onCopyDirectory: () => void;
  children: ReactNode;
}) {
  const [collapsed, setCollapsed] = useState(true);
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
      /* Appearance persistence is optional; never affect gateway state. */
    }
  }, [dark]);
  const navigate = (next: Workspace) => {
    onTab(next);
  };
  const item = (entry: (typeof navigation)[number]) => (
    <Tooltip key={entry.id}>
      <TooltipTrigger asChild>
        <button
          type="button"
          className="mg-nav-item"
          aria-label={entry.label}
          aria-current={tab === entry.id ? "page" : undefined}
          onClick={() => navigate(entry.id)}
        >
          <entry.icon size={17} aria-hidden />
          <span>{entry.name}</span>
        </button>
      </TooltipTrigger>
      {collapsed && (
        <TooltipContent side="right" className="mg-nav-tooltip">
          {entry.name}
        </TooltipContent>
      )}
    </Tooltip>
  );
  return (
    <TooltipProvider delayDuration={250}>
      <div
        className={`mg-app mg-dashboard-client ${collapsed ? "mg-sidebar-collapsed" : "mg-sidebar-expanded"}`}
      >
        <a className="mg-skip-link" href="#manual-content">
          跳到工作区内容
        </a>
        <div className="mg-dragbar" data-tauri-drag-region />
        <div className="mg-layout">
          <aside
            className="mg-sidebar"
            id="manual-sidebar"
            aria-label="侧边导航"
          >
            <div className="mg-sidebar-top">
              <button
                className="mg-brand"
                type="button"
                aria-label="LumaGate 概览"
                title="枢光 · LumaGate"
                onClick={() => navigate("overview")}
              >
                <span className="mg-brand-mark">
                  <img src={brandMark} alt="" aria-hidden />
                </span>
                <strong>LumaGate</strong>
              </button>
              <ActionButton
                variant="ghost"
                size="icon"
                className="mg-sidebar-toggle"
                aria-label={collapsed ? "展开侧栏" : "收起侧栏"}
                title={collapsed ? "展开侧栏" : "收起侧栏"}
                aria-controls="manual-sidebar"
                aria-expanded={!collapsed}
                onClick={() => setCollapsed((v) => !v)}
              >
                {collapsed ? (
                  <PanelLeftOpen size={18} />
                ) : (
                  <PanelLeftClose size={18} />
                )}
              </ActionButton>
            </div>
            <div className="mg-sidebar-content">
              <nav aria-label="工作区">
                {navigation.slice(0, 5).map((entry, i) => (
                  <div key={entry.id}>
                    {i === 2 && (
                      <span className="mg-nav-group-label">配置</span>
                    )}
                    {item(entry)}
                  </div>
                ))}
              </nav>
              <div className="mg-sidebar-foot">
                <nav aria-label="辅助导航">{navigation.slice(5).map(item)}</nav>
                <div className="mg-sidebar-gateway">
                  <span
                    className={`mg-status ${snapshot?.status.running ? "mg-status--running" : ""}`}
                  >
                    <i aria-hidden />
                    {!snapshot
                      ? "状态待读取"
                      : snapshot.status.running
                        ? "网关运行中"
                        : "网关已停止"}
                  </span>
                  <code>{snapshot?.baseUrl ?? "—"}</code>
                  <small>枢光 · {appVersion}</small>
                </div>
              </div>
            </div>
          </aside>
          <section className="mg-workspace">
            <header className="mg-header" data-tauri-drag-region>
              <span className="mg-breadcrumb">
                {navigation.find((n) => n.id === tab)?.name}
              </span>
              <div className="mg-header-actions">
                <span
                  className={`mg-header-status ${snapshot?.status.running ? "is-running" : ""}`}
                >
                  <i aria-hidden />
                  {!snapshot
                    ? "状态待读取"
                    : snapshot.status.running
                      ? "网关运行中"
                      : "网关已停止"}
                </span>
                <ActionButton
                  variant="ghost"
                  size="icon"
                  disabled={!snapshot}
                  aria-label="复制目录地址"
                  title="复制目录地址"
                  onClick={onCopyDirectory}
                >
                  <Copy size={15} />
                </ActionButton>
                <ActionButton
                  variant="ghost"
                  size="icon"
                  aria-label="切换外观"
                  title="切换外观"
                  onClick={() => setDark((v) => !v)}
                >
                  {dark ? <Sun size={16} /> : <Moon size={16} />}
                </ActionButton>
                <ActionButton
                  size="sm"
                  disabled={
                    busy ||
                    !snapshot ||
                    (!snapshot.status.running &&
                      snapshot.logStorage?.ready === false)
                  }
                  variant={snapshot?.status.running ? "outline" : "default"}
                  onClick={onGateway}
                >
                  {snapshot?.status.running ? (
                    <Square size={12} />
                  ) : (
                    <Play size={12} />
                  )}
                  {snapshot?.status.running ? "停止网关" : "启动网关"}
                </ActionButton>
              </div>
            </header>
            <main
              className={`mg-main ${tab === "models" && snapshot ? "mg-main--catalog" : ""}`}
              id="manual-content"
              tabIndex={0}
            >
              {children}
            </main>
          </section>
        </div>
      </div>
    </TooltipProvider>
  );
}
