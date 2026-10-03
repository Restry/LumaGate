import { useEffect, useState, type FormEvent } from "react";
import { Copy, RefreshCw } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Input } from "@/components/ui/input";
import { ActionButton, Choice, Field } from "./ui";
import { AccessKeys } from "./AccessKeys";
import type {
  AccessKeyChangeResult,
  AccessKeyOperation,
  GatewayNetworkSettings,
  GatewayNetworkSnapshot,
  GatewayNetworkUpdate,
} from "./types";
import "./network-settings.css";

export function NetworkSettings({
  network,
  busy,
  onSave,
  onCopy,
  onRefresh,
  onKeyChange,
  onCopyKey,
  onRevealKey,
}: {
  network: GatewayNetworkSnapshot;
  busy: boolean;
  onSave: (
    settings: GatewayNetworkUpdate,
  ) => Promise<GatewayNetworkSettings | undefined>;
  onCopy: (value: string, label: string) => void;
  onRefresh: () => void;
  onKeyChange: (
    revision: number,
    operation: AccessKeyOperation,
  ) => Promise<AccessKeyChangeResult>;
  onCopyKey: (id: string) => Promise<void>;
  onRevealKey: (id: string) => Promise<string>;
}) {
  const [basis, setBasis] = useState(network.saved);
  const [address, setAddress] = useState(network.saved.listenAddress);
  const [port, setPort] = useState(String(network.saved.listenPort));
  const [errors, setErrors] = useState<{ port?: string; keys?: string }>({});
  const [message, setMessage] = useState("");
  const dirty =
    address !== basis.listenAddress || port !== String(basis.listenPort);
  const effective = network.active ?? network.saved;
  const lan = address === "0.0.0.0";
  const hasEnabledKey = network.accessKeys.items.some((key) => key.enabled);
  const conflict = network.saved.revision > basis.revision;
  function reset(saved: GatewayNetworkSettings) {
    setBasis(saved);
    setAddress(saved.listenAddress);
    setPort(String(saved.listenPort));
    setErrors({});
    setMessage("");
  }
  useEffect(() => {
    if (!dirty && network.saved.revision > basis.revision) reset(network.saved);
  }, [network.saved, basis.revision, dirty]);
  async function submit(event: FormEvent) {
    event.preventDefault();
    const nextErrors: typeof errors = {};
    if (!/^\d+$/.test(port.trim()) || Number(port) < 1 || Number(port) > 65535)
      nextErrors.port = "请输入 1–65535 之间的整数端口。";
    if (lan && !hasEnabledKey)
      nextErrors.keys = "请先添加并启用至少一把访问密钥，再允许内网访问。";
    setErrors(nextErrors);
    setMessage("");
    if (nextErrors.port || nextErrors.keys) {
      document
        .getElementById(
          nextErrors.port ? "network-port" : "gateway-add-access-key",
        )
        ?.focus();
      return;
    }
    const saved = await onSave({
      revision: basis.revision,
      listenAddress: address,
      listenPort: Number(port),
    });
    if (saved) {
      reset(saved);
      setMessage("监听设置已保存；保存操作不会重启网关或改写 Agent 配置。");
    }
  }
  return (
    <section className="mg-network-settings">
      <div className="mg-page-heading">
        <div>
          <h1>设置</h1>
          <p>监听变更需手动重启；访问密钥可单独管理，对后续请求即时生效。</p>
        </div>
        <ActionButton variant="outline" disabled={busy} onClick={onRefresh}>
          <RefreshCw size={14} aria-hidden />
          刷新设置
        </ActionButton>
      </div>
      {network.restartRequired && (
        <Alert className="mg-network-pending">
          <AlertDescription>
            <strong>已保存，等待手动重启网关</strong>
            <p>
              当前仍监听 {network.active?.listenAddress}:
              {network.active?.listenPort}
              。请等待请求结束后，用顶部按钮停止，再启动网关。切回仅本机也要重启监听才生效；密钥新增、停用和删除无需重启。
            </p>
            <p>
              停止会中断仍在进行的请求，但保留已保存日志。端口变化后，请重新预览并确认
              Agent 同步。
            </p>
          </AlertDescription>
        </Alert>
      )}
      {network.active?.allowLan && !hasEnabledKey && (
        <Alert className="mg-network-pending">
          <AlertDescription>
            内网监听仍在运行，但没有启用的访问密钥，远程请求会被拒绝。添加或启用密钥即可恢复；本机免鉴权请求不受影响。
          </AlertDescription>
        </Alert>
      )}
      <div className="mg-network-grid">
        <div className="mg-network-controls">
          <AccessKeys
            keys={network.accessKeys}
            busy={busy}
            onChange={onKeyChange}
            onCopySaved={onCopyKey}
            onReveal={onRevealKey}
          />
          <form
            className="mg-network-form"
            aria-label="网关网络设置"
            onSubmit={(event) => void submit(event)}
            noValidate
          >
            <section
              className="mg-network-section"
              aria-labelledby="network-listener-heading"
            >
              <h2 id="network-listener-heading">监听与访问范围</h2>
              <Field id="network-address" label="监听地址">
                <Choice
                  id="network-address"
                  value={address}
                  onChange={setAddress}
                  disabled={busy}
                  options={[
                    {
                      value: "127.0.0.1",
                      label: "127.0.0.1 · 仅本机访问（默认）",
                    },
                    { value: "0.0.0.0", label: "0.0.0.0 · 允许内网访问" },
                  ]}
                />
                <p className="mg-hint">
                  {lan
                    ? "允许 IPv4 内网设备持密钥访问。HTTP 不加密，仅用于可信内网；不要公网映射。0.0.0.0 不是客户端连接地址。"
                    : "只有这台电脑上的程序能调用；其他设备无法通过内网 IP 连接。"}
                </p>
              </Field>
              <Field id="network-port" label="监听端口">
                <Input
                  id="network-port"
                  name="gatewayPort"
                  type="text"
                  inputMode="numeric"
                  autoComplete="off"
                  value={port}
                  onChange={(event) => setPort(event.target.value)}
                  disabled={busy}
                  className="mg-network-input"
                  aria-invalid={!!errors.port}
                  aria-describedby={
                    errors.port ? "network-port-error" : "network-port-hint"
                  }
                />
                <p className="mg-hint" id="network-port-hint">
                  默认
                  15722。端口占用或权限不足会在手动启动时提示，不会自动改用其他端口。
                </p>
                {errors.port && (
                  <p className="mg-network-error" id="network-port-error">
                    {errors.port}
                  </p>
                )}
              </Field>
            </section>
            {errors.keys && !hasEnabledKey && (
              <p role="alert" className="mg-network-error">
                {errors.keys}
              </p>
            )}
            {conflict && (
              <Alert>
                <AlertDescription>
                  服务器监听设置已更新，草稿没有被覆盖。请先保留需要的内容，再点击“放弃修改”载入最新设置。
                </AlertDescription>
              </Alert>
            )}
            <div className="mg-network-actions">
              <ActionButton type="submit" disabled={busy} aria-busy={busy}>
                保存网络设置
              </ActionButton>
              {dirty && (
                <ActionButton
                  type="button"
                  variant="outline"
                  disabled={busy}
                  onClick={() => reset(network.saved)}
                >
                  放弃修改
                </ActionButton>
              )}
              {dirty && <span className="mg-hint">有未保存的监听修改</span>}
            </div>
            <p className="mg-network-message" role="status">
              {message}
            </p>
          </form>
        </div>
        <aside
          className="mg-network-connection"
          aria-labelledby="network-current-heading"
        >
          <section className="mg-network-section">
            <h2 id="network-current-heading">
              {network.active ? "当前生效的连接" : "下次启动使用的连接"}
            </h2>
            <dl className="mg-network-summary">
              <div>
                <dt>服务状态</dt>
                <dd>{network.active ? "监听中" : "已停止，地址尚不可用"}</dd>
              </div>
              <div>
                <dt>访问范围</dt>
                <dd>
                  {effective.allowLan ? "本机 + 持密钥的内网设备" : "仅本机"}
                </dd>
              </div>
              <div>
                <dt>监听地址</dt>
                <dd>
                  <code>
                    {effective.listenAddress}:{effective.listenPort}
                  </code>
                </dd>
              </div>
              <div>
                <dt>启用的访问密钥</dt>
                <dd>
                  {network.accessKeys.items.filter((key) => key.enabled).length}{" "}
                  把
                </dd>
              </div>
            </dl>
            <div className="mg-network-address">
              <h3>本机 Base URL</h3>
              <code>{`http://127.0.0.1:${effective.listenPort}/v1`}</code>
              <ActionButton
                type="button"
                size="sm"
                variant="outline"
                onClick={() =>
                  onCopy(
                    `http://127.0.0.1:${effective.listenPort}/v1`,
                    "本机 Base URL",
                  )
                }
              >
                <Copy size={13} aria-hidden />
                复制本机地址
              </ActionButton>
            </div>
          </section>
          <section className="mg-network-section">
            <h2>
              {network.active?.allowLan
                ? "内网 Base URL"
                : "内网候选地址（尚未开放）"}
            </h2>
            <p className="mg-network-note">
              选择另一台设备能访问的网卡地址。以下地址来自本机网卡，不代表已经验证可达。
            </p>
            {network.lanAddresses.map((url) => (
              <div className="mg-network-address" key={url}>
                <code>{url}</code>
                <ActionButton
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => onCopy(url, "内网 Base URL")}
                  aria-label={`复制内网地址 ${url}`}
                >
                  <Copy size={13} aria-hidden />
                  复制地址
                </ActionButton>
              </div>
            ))}
            {!network.lanAddresses.length && (
              <p className="mg-network-note">
                {network.addressError ||
                  "未识别到内网 IPv4。请检查 Wi-Fi、有线网络或 VPN，并刷新设置。"}
              </p>
            )}
            <details className="mg-network-example">
              <summary>如何在另一台设备接入</summary>
              <p>
                OpenAI 兼容客户端填写上面的 Base URL
                和任意一把启用的访问密钥；Anthropic 客户端使用去掉 /v1
                的服务地址，并通过 x-api-key 传入密钥。
              </p>
              <p>
                先在客户端安全设置环境变量
                LUMAGATE_LAN_KEY，再手动读取目录（不调用模型）：
              </p>
              <pre>
                <code>{`curl --fail '${network.lanAddresses[0] ?? `http://<本机内网IPv4>:${effective.listenPort}/v1`}/models' \\\n  -H "Authorization: Bearer $LUMAGATE_LAN_KEY"`}</code>
              </pre>
            </details>
          </section>
          <p className="mg-network-security">
            仅用于可信内网：HTTP
            不加密密钥和请求。请勿配置公网端口映射或不可信反向代理。防火墙、VPN
            和 Wi-Fi
            客户端隔离可能阻止连接；本应用不会自动更改它们，也不开放网页跨源调用。
          </p>
        </aside>
      </div>
    </section>
  );
}
