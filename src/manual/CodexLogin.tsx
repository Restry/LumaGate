import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CheckCircle2, ExternalLink, Laptop, RefreshCw } from "lucide-react";
import { ActionButton, Modal } from "./ui";
import type { CodexAuthStatus } from "./types";
import "./copilot.css";

interface Challenge {
  flowId: string;
  userCode: string;
  verificationUri: string;
  expiresIn: number;
  interval: number;
}
interface Poll {
  state: "pending" | "connected";
  interval?: number;
  providerId?: string;
  email?: string | null;
}

export function CodexLogin({
  status,
  onClose,
  onConnected,
  onDisconnect,
}: {
  status?: CodexAuthStatus;
  onClose: () => void;
  onConnected: (providerId: string) => Promise<void>;
  onDisconnect?: () => void;
}) {
  const [phase, setPhase] = useState<
    "choose" | "importing" | "starting" | "waiting" | "discovering" | "closing" | "error"
  >("choose");
  const [challenge, setChallenge] = useState<Challenge | null>(null);
  const [error, setError] = useState("");
  const [copyHint, setCopyHint] = useState("");
  const [remaining, setRemaining] = useState(0);
  const [opening, setOpening] = useState(false);

  const alive = useRef(true);
  const flow = useRef<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout>>();
  const clock = useRef<ReturnType<typeof setInterval>>();
  const generation = useRef(0);

  const clear = () => {
    clearTimeout(timer.current);
    clearInterval(clock.current);
  };

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      generation.current++;
      clear();
      if (flow.current) {
        void invoke("manual_codex_cancel", { flowId: flow.current }).catch(
          () => {},
        );
      }
    };
  }, []);

  const close = async () => {
    generation.current++;
    clear();
    if (flow.current) {
      setPhase("closing");
      try {
        await invoke("manual_codex_cancel", { flowId: flow.current });
        flow.current = null;
      } catch {
        if (alive.current) {
          setError("取消授权失败，请重试。");
          setPhase("error");
        }
        return;
      }
    }
    onClose();
  };

  const importLocal = async () => {
    setError("");
    setPhase("importing");
    try {
      const result = await invoke<{ providerId: string }>(
        "manual_codex_import_local",
      );
      if (!alive.current) return;
      setPhase("discovering");
      await onConnected(result.providerId);
      if (alive.current) onClose();
    } catch (e) {
      if (alive.current) {
        setError(String(e));
        setPhase("error");
      }
    }
  };

  const startDevice = async () => {
    const run = ++generation.current;
    clear();
    setError("");
    setCopyHint("");
    setPhase("starting");
    setChallenge(null);
    if (flow.current) {
      void invoke("manual_codex_cancel", { flowId: flow.current }).catch(
        () => {},
      );
      flow.current = null;
    }
    try {
      const next = await invoke<Challenge>("manual_codex_start");
      if (!alive.current || run !== generation.current) {
        void invoke("manual_codex_cancel", { flowId: next.flowId }).catch(
          () => {},
        );
        return;
      }
      flow.current = next.flowId;
      setChallenge(next);
      setRemaining(next.expiresIn);
      setPhase("waiting");
      const until = Date.now() + next.expiresIn * 1000;
      const poll = async () => {
        if (!alive.current || run !== generation.current) return;
        try {
          const result = await invoke<Poll>("manual_codex_poll", {
            flowId: next.flowId,
          });
          if (!alive.current || run !== generation.current) return;
          if (result.state === "connected" && result.providerId) {
            clear();
            flow.current = null;
            setPhase("discovering");
            await onConnected(result.providerId);
            if (alive.current) onClose();
          } else {
            timer.current = setTimeout(
              () => void poll(),
              Math.max(5, result.interval ?? next.interval) * 1000,
            );
          }
        } catch (e) {
          clear();
          if (alive.current && run === generation.current) {
            setError(String(e));
            setPhase("error");
          }
        }
      };
      timer.current = setTimeout(
        () => void poll(),
        Math.max(5, next.interval) * 1000,
      );
      clock.current = setInterval(() => {
        const left = Math.max(0, Math.ceil((until - Date.now()) / 1000));
        setRemaining(left);
        if (!left) {
          generation.current++;
          clear();
          void invoke("manual_codex_cancel", { flowId: next.flowId }).catch(
            () => {},
          );
          flow.current = null;
          setPhase("error");
          setError("授权码已过期，请重新生成。");
        }
      }, 1000);
    } catch (e) {
      if (alive.current && run === generation.current) {
        setError(String(e));
        setPhase("error");
      }
    }
  };

  const openLogin = async () => {
    if (!challenge || opening) return;
    setOpening(true);
    let copied = false;
    try {
      await navigator.clipboard.writeText(challenge.userCode);
      copied = true;
    } catch {
      /* fallback selectable */
    }
    try {
      await invoke("manual_codex_open_login");
      if (alive.current) {
        setCopyHint(
          copied
            ? "代码已复制，请在浏览器粘贴完成授权"
            : "请复制上方代码并在 OpenAI 页面粘贴",
        );
      }
    } catch {
      if (alive.current) {
        setCopyHint("请手动访问 https://auth.openai.com/codex/device");
      }
    } finally {
      if (alive.current) setOpening(false);
    }
  };

  const busy =
    phase === "importing" ||
    phase === "starting" ||
    phase === "discovering" ||
    phase === "closing";

  return (
    <Modal
      title="连接 Codex 订阅"
      description="将你的 ChatGPT Plus / Pro 订阅接入 LumaGate 作为模型来源。"
      className="mc-copilot-dialog"
      busy={busy}
      onClose={() => void close()}
      footer={
        <>
          {status?.connected && onDisconnect && phase === "choose" && (
            <ActionButton
              variant="outline"
              disabled={busy}
              onClick={() => {
                onClose();
                onDisconnect();
              }}
            >
              断开订阅
            </ActionButton>
          )}
          <ActionButton
            variant="outline"
            disabled={busy}
            onClick={() => void close()}
          >
            {phase === "discovering" ? "正在拉取模型…" : "关闭"}
          </ActionButton>
          {phase === "waiting" && (
            <ActionButton disabled={opening} onClick={() => void openLogin()}>
              <ExternalLink size={14} />
              {opening ? "正在打开…" : "复制代码并打开 OpenAI"}
            </ActionButton>
          )}
          {phase === "error" && (
            <ActionButton onClick={() => setPhase("choose")}>
              <RefreshCw size={14} />
              返回重试
            </ActionButton>
          )}
        </>
      }
    >
      <div className="mc-copilot-login">
        {phase === "choose" && (
          <div className="lg-oauth-options">
            {status?.connected && (
              <div className="lg-oauth-connected">
                <CheckCircle2 size={16} aria-hidden />
                <div>
                  <strong>
                    已连接 {status.email || status.accountId || "ChatGPT 账号"}
                  </strong>
                  <span>
                    {status.plan ? `${status.plan.toUpperCase()} 订阅 · ` : ""}
                    {status.source === "local_codex"
                      ? "来自本机 Codex 登录"
                      : "来自浏览器设备授权"}
                  </span>
                </div>
              </div>
            )}

            <div className="lg-oauth-option is-primary">
              <div className="lg-oauth-option-head">
                <Laptop size={16} aria-hidden />
                <div>
                  <strong>使用本机 Codex 登录</strong>
                  <span>
                    {status?.localAvailable
                      ? `已检测到本机账号${status.localEmail ? `（${status.localEmail}）` : ""}，直接导入即可`
                      : "读取 ~/.codex/auth.json 中的 ChatGPT 订阅令牌"}
                  </span>
                </div>
              </div>
              <ActionButton
                size="sm"
                disabled={!status?.localAvailable}
                onClick={() => void importLocal()}
              >
                {status?.connected ? "重新导入" : "一键导入"}
              </ActionButton>
            </div>

            <div className="lg-oauth-option">
              <div className="lg-oauth-option-head">
                <ExternalLink size={16} aria-hidden />
                <div>
                  <strong>浏览器设备码登录</strong>
                  <span>在 OpenAI 网页登录或切换其他 ChatGPT 订阅账号</span>
                </div>
              </div>
              <ActionButton
                size="sm"
                variant="outline"
                onClick={() => void startDevice()}
              >
                去授权
              </ActionButton>
            </div>
          </div>
        )}

        {phase === "importing" && <p role="status">正在导入本机 Codex 订阅…</p>}
        {phase === "starting" && <p role="status">正在生成设备授权码…</p>}

        {phase === "waiting" && challenge && (
          <section aria-label="OpenAI 设备授权">
            <code
              className="mc-device-code"
              tabIndex={0}
              aria-label="OpenAI 设备授权码"
            >
              {challenge.userCode}
            </code>
            <div className="mc-copilot-status">
              <p role="status">{copyHint || "等待 OpenAI 浏览器授权"}</p>
              <span aria-label="授权码剩余时间">
                {Math.floor(remaining / 60)}:
                {String(remaining % 60).padStart(2, "0")}
              </span>
            </div>
          </section>
        )}

        {phase === "discovering" && (
          <p role="status">
            <RefreshCw size={15} />
            已连接，正在拉取订阅可用模型…
          </p>
        )}

        {error && (
          <p className="mc-notice" role="alert">
            {error}
          </p>
        )}
      </div>
    </Modal>
  );
}
