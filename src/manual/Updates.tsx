import { useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Switch } from "@/components/ui/switch";
import { ActionButton, Modal } from "./ui";
import "./updates.css";

type Phase =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "ready"
  | "waiting"
  | "installing"
  | "error"
  | "unsupported";
export type UpdateView = {
  phase: Phase;
  currentVersion: string;
  version: string | null;
  notes: string;
  autoCheck: boolean;
  prompt: boolean;
  received: number;
  total: number | null;
  active: number;
  message: string;
  checkedAt: string | null;
};
const busyPhase = (phase: Phase) =>
  ["checking", "downloading", "waiting", "installing"].includes(phase);
const megabytes = (bytes: number) => `${(bytes / 1024 / 1024).toFixed(1)} MB`;

export function Updates({ settingsVisible }: { settingsVisible: boolean }) {
  const [view, setView] = useState<UpdateView | null>(null);
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const submitting = useRef(false);
  useEffect(() => {
    // The isolated browser preview has no native updater and must never contact releases.
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let revision = 0;
    void (async () => {
      unlisten = await listen<UpdateView>("manual-update", ({ payload }) => {
        revision += 1;
        if (!disposed) setView(payload);
      });
      if (disposed) {
        unlisten();
        return;
      }
      const started = revision;
      const state = await invoke<UpdateView>("manual_update_state");
      if (!disposed && started === revision) setView(state);
    })().catch((reason: unknown) => {
      if (!disposed) setError(`无法读取更新状态：${String(reason)}`);
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
  async function command(name: string, args?: Record<string, unknown>) {
    // Cancel remains available while the native installation request awaits drain.
    const cancel = name === "manual_update_later";
    if (submitting.current && !cancel) return;
    if (!cancel) {
      submitting.current = true;
      setPending(true);
    }
    setError("");
    try {
      await invoke(name, args);
      setView(await invoke<UpdateView>("manual_update_state"));
    } catch (reason) {
      setError(String(reason));
    } finally {
      if (!cancel) {
        submitting.current = false;
        setPending(false);
      }
    }
  }
  const busy = view ? busyPhase(view.phase) : false;
  const canInstall = view?.version && !busy && view.phase !== "unsupported";
  const later = () => void command("manual_update_later");
  const install = () => void command("manual_update_install");
  const progress =
    view &&
    (view.phase === "downloading" ||
      view.phase === "waiting" ||
      view.phase === "installing") ? (
      <div className="mg-update-progress">
        {view.phase === "downloading" && (
          <>
            <progress
              aria-label="更新下载进度"
              max={view.total || undefined}
              value={view.total ? view.received : undefined}
            />
            <span>
              {megabytes(view.received)}
              {view.total ? ` / ${megabytes(view.total)}` : ""}
            </span>
          </>
        )}
        <p role="status">{view.message}</p>
      </div>
    ) : (
      <p role="status">{view?.message}</p>
    );
  return (
    <>
      <section
        className="mg-updates"
        hidden={!settingsVisible}
        aria-labelledby="update-heading"
      >
        <div className="mg-update-heading">
          <div>
            <h2 id="update-heading">应用更新</h2>
            <p>
              {view ? `当前版本 ${view.currentVersion}` : "更新状态尚未就绪"}
            </p>
          </div>
          <ActionButton
            variant="outline"
            disabled={!view || busy || pending}
            onClick={() => void command("manual_update_check")}
          >
            {view?.phase === "checking" ? "正在检查…" : "检查更新"}
          </ActionButton>
        </div>
        <div className="mg-update-preference">
          <label htmlFor="auto-update-check">
            自动检查更新<span>启动后及每 6 小时检查；未经确认不下载。</span>
          </label>
          <Switch
            id="auto-update-check"
            checked={view?.autoCheck ?? true}
            disabled={!view || pending || view.phase === "installing"}
            onCheckedChange={(enabled) =>
              void command("manual_update_auto", { enabled })
            }
          />
        </div>
        {progress}
        {error && (
          <p role="alert" className="mg-network-error">
            {error}
          </p>
        )}
        {view?.checkedAt && (
          <p>上次检查：{new Date(view.checkedAt).toLocaleString()}</p>
        )}
        {canInstall && (
          <ActionButton disabled={pending} onClick={install}>
            {view.phase === "ready" ? "安装并重启" : "下载并更新"}
            {` ${view.version}`}
          </ActionButton>
        )}
        <p>
          仅从 Restry/LumaGate
          正式发布下载并验签。安装前等待当前请求结束，重启会短暂中断服务；已停止的网关不会自动启动。
        </p>
        <p>
          Linux 应用内安装仅支持 AppImage；deb 请使用包管理器或官方安装包升级。
        </p>
      </section>
      {view?.prompt && (
        <Modal
          title={`更新至 LumaGate ${view.version ?? ""}`}
          description={`当前版本 ${view.currentVersion}。确认后自动下载并验签，等待现有请求结束后安装重启。`}
          busy={view.phase === "installing"}
          onClose={later}
          footer={
            <>
              <ActionButton
                variant="outline"
                disabled={view.phase === "installing"}
                onClick={later}
              >
                {view.phase === "waiting"
                  ? "取消更新，继续服务"
                  : view.phase === "downloading"
                    ? "取消下载"
                    : "稍后"}
              </ActionButton>
              <ActionButton disabled={!canInstall || pending} onClick={install}>
                {view.phase === "ready"
                  ? "安装并重启"
                  : view.phase === "error"
                    ? "重试更新"
                    : busy
                      ? "更新处理中…"
                      : "下载并更新"}
              </ActionButton>
            </>
          }
        >
          {progress}
          {error && (
            <p role="alert" className="mg-network-error">
              {error}
            </p>
          )}
          {view.notes && (
            <details
              className="mg-update-notes"
              open={view.phase === "available"}
            >
              <summary>更新说明</summary>
              <pre>{view.notes}</pre>
            </details>
          )}
          <p>
            排空期间新请求返回可重试的
            503；可取消恢复接收请求。不会中断正在生成的回答。
          </p>
        </Modal>
      )}
    </>
  );
}
