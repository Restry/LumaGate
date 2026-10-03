import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ExternalLink, RefreshCw } from "lucide-react";
import { ActionButton, Modal } from "./ui";
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
  login?: string;
}
export function CopilotLogin({
  onClose,
  onConnected,
}: {
  onClose: () => void;
  onConnected: (providerId: string) => Promise<void>;
}) {
  const [phase, setPhase] = useState<
      "idle" | "starting" | "waiting" | "discovering" | "closing" | "error"
    >("starting"),
    [challenge, setChallenge] = useState<Challenge | null>(null),
    [error, setError] = useState(""),
    [copyHint, setCopyHint] = useState(""),
    [remaining, setRemaining] = useState(0),
    [opening, setOpening] = useState(false);
  const alive = useRef(true),
    flow = useRef<string | null>(null),
    timer = useRef<ReturnType<typeof setTimeout>>(),
    clock = useRef<ReturnType<typeof setInterval>>(),
    generation = useRef(0);
  const clear = () => {
    clearTimeout(timer.current);
    clearInterval(clock.current);
  };
  useEffect(() => {
    alive.current = true;
    // The user already chose Connect. Defer once so StrictMode's probe cannot
    // create two GitHub device codes; cleanup cancels the probe's task.
    const boot = setTimeout(() => void start(), 0);
    return () => {
      clearTimeout(boot);
      alive.current = false;
      generation.current++;
      clear();
      if (flow.current)
        void invoke("manual_copilot_cancel", { flowId: flow.current }).catch(
          () => {},
        );
    };
  }, []);
  const close = async () => {
    generation.current++;
    clear();
    if (flow.current) {
      setPhase("closing");
      try {
        await invoke("manual_copilot_cancel", { flowId: flow.current });
        flow.current = null;
      } catch {
        if (alive.current) {
          setError("无法确认取消授权，请重试取消。");
          setPhase("error");
        }
        return;
      }
    }
    onClose();
  };
  const start = async () => {
    const run = ++generation.current;
    clear();
    setError("");
    setCopyHint("");
    setPhase("starting");
    setChallenge(null);
    if (flow.current) {
      void invoke("manual_copilot_cancel", { flowId: flow.current }).catch(
        () => {},
      );
      flow.current = null;
    }
    try {
      const next = await invoke<Challenge>("manual_copilot_start");
      if (!alive.current || run !== generation.current) {
        void invoke("manual_copilot_cancel", { flowId: next.flowId }).catch(
          () => {},
        );
        return;
      }
      flow.current = next.flowId;
      setChallenge(next);
      setRemaining(next.expiresIn);
      setPhase("waiting");
      const until = Date.now() + next.expiresIn * 1000;
      const fail = (text: string) => {
        clear();
        if (alive.current && run === generation.current) {
          setError(text);
          setPhase("error");
        }
      };
      const poll = async () => {
        if (!alive.current || run !== generation.current) return;
        try {
          const result = await invoke<Poll>("manual_copilot_poll", {
            flowId: next.flowId,
          });
          if (!alive.current || run !== generation.current) return;
          if (result.state === "connected" && result.providerId) {
            clear();
            flow.current = null;
            setPhase("discovering");
            await onConnected(result.providerId);
          } else
            timer.current = setTimeout(
              () => void poll(),
              Math.max(5, result.interval ?? next.interval) * 1000,
            );
        } catch (e) {
          fail(String(e));
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
          void invoke("manual_copilot_cancel", { flowId: next.flowId }).catch(
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
      /* Still open the page; the code remains selectable. */
    }
    try {
      await invoke("manual_copilot_open_login");
      if (alive.current)
        setCopyHint(
          copied
            ? "代码已复制，等待 GitHub 授权"
            : "请手动复制上方代码，在 GitHub 粘贴",
        );
    } catch {
      if (alive.current)
        setCopyHint("无法打开浏览器，请访问 https://github.com/login/device");
    } finally {
      if (alive.current) setOpening(false);
    }
  };
  return (
    <Modal
      title="连接 GitHub Copilot"
      description="在 GitHub 输入设备码完成登录。"
      className="mc-copilot-dialog"
      busy={phase === "discovering" || phase === "closing"}
      onClose={() => void close()}
      footer={
        <>
          <ActionButton
            variant="outline"
            disabled={phase === "discovering" || phase === "closing"}
            onClick={() => void close()}
          >
            {phase === "discovering" ? "正在获取模型…" : "取消"}
          </ActionButton>
          {phase === "waiting" && (
            <ActionButton disabled={opening} onClick={() => void openLogin()}>
              <ExternalLink size={14} />
              {opening ? "正在打开…" : "复制代码并打开 GitHub"}
            </ActionButton>
          )}
          {phase === "error" && (
            <ActionButton onClick={() => void start()}>
              <RefreshCw size={14} />
              重新生成授权码
            </ActionButton>
          )}
        </>
      }
    >
      <div className="mc-copilot-login">
        {phase === "starting" && <p role="status">正在生成设备码…</p>}
        {phase === "waiting" && challenge && (
          <section aria-label="GitHub 设备授权">
            <code
              className="mc-device-code"
              tabIndex={0}
              aria-label="GitHub 设备授权码"
            >
              {challenge.userCode}
            </code>
            <div className="mc-copilot-status">
              <p role="status">{copyHint || "等待 GitHub 授权"}</p>
              <span aria-label="授权码剩余时间">
                {Math.floor(remaining / 60)}:
                {String(remaining % 60).padStart(2, "0")}
              </span>
            </div>
            <small>授权页可能显示 VS Code。</small>
          </section>
        )}
        {phase === "discovering" && (
          <p role="status">
            <RefreshCw size={15} />
            已登录，正在获取模型…
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
