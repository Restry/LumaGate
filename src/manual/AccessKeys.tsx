import { useId, useRef, useState, type FormEvent } from "react";
import { Copy, KeyRound, Plus } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { ActionButton, Field, Modal } from "./ui";
import type {
  AccessKeyChangeResult,
  AccessKeyOperation,
  GatewayAccessKey,
  GatewayAccessKeys,
} from "./types";

type Editor = {
  kind: "add" | "rename" | "restore_copy" | "remove";
  key?: GatewayAccessKey;
  revision: number;
};
const errorText = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

export function AccessKeys({
  keys,
  busy,
  onChange,
  onCopySaved,
  onReveal,
}: {
  keys: GatewayAccessKeys;
  busy: boolean;
  onChange: (
    revision: number,
    operation: AccessKeyOperation,
  ) => Promise<AccessKeyChangeResult>;
  onCopySaved: (id: string) => Promise<void>;
  onReveal: (id: string) => Promise<string>;
}) {
  const [confirmed, setConfirmed] = useState(keys);
  const current = confirmed.revision > keys.revision ? confirmed : keys;
  const [editor, setEditor] = useState<Editor | null>(null);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [copying, setCopying] = useState<string | null>(null);
  const copyPending = useRef(false);
  const [manualCopy, setManualCopy] = useState<string | null>(null);
  async function change(revision: number, operation: AccessKeyOperation) {
    setError("");
    const result = await onChange(revision, operation);
    setConfirmed(result.keys);
    setMessage(
      result.warnings.length
        ? result.warnings.join("；")
        : "密钥变更已保存，对后续请求即时生效，无需重启网关。",
    );
    return result;
  }
  async function copy(key: GatewayAccessKey) {
    if (copyPending.current) return;
    copyPending.current = true;
    setCopying(key.id);
    setError("");
    try {
      await onCopySaved(key.id);
      setMessage(`已复制“${key.name}”的访问密钥。`);
    } catch {
      try {
        setManualCopy(await onReveal(key.id));
      } catch (reason) {
        setError(errorText(reason));
      }
    } finally {
      copyPending.current = false;
      setCopying(null);
    }
  }
  return (
    <section className="mg-access-keys" aria-labelledby="access-keys-heading">
      <div className="mg-access-keys-heading">
        <h2 id="access-keys-heading">
          访问密钥 <span>{current.items.length} / 64</span>
        </h2>
        <ActionButton
          id="gateway-add-access-key"
          type="button"
          variant="outline"
          disabled={busy}
          onClick={() => setEditor({ kind: "add", revision: current.revision })}
        >
          <Plus size={14} aria-hidden />
          添加密钥
        </ActionButton>
      </div>
      <p className="mg-network-note">
        为不同设备或用途分别命名，保存后可随时复制。新增、停用和删除立即影响后续请求，已开始的响应不会被打断。
      </p>
      <p className="mg-hint">
        为支持重复复制，明文保存在本机私有 access-keys 文件夹（macOS/Unix 0700 /
        0600），不进入列表快照或日志。本机请求携带有效密钥时也会标注名称；未使用有效密钥的本机请求仍免鉴权。
      </p>
      {!current.items.length && (
        <p className="mg-network-note">
          还没有访问密钥。允许内网访问前，请添加并启用至少一把。
        </p>
      )}
      <ul className="mg-access-key-list" aria-label="已保存的访问密钥">
        {current.items.map((key) => (
          <li key={key.id} className="mg-access-key-row">
            <div className="mg-access-key-identity">
              <div>
                <strong>{key.name}</strong>
                <span>{key.enabled ? "已启用" : "已停用"}</span>
              </div>
              <p>
                {key.canCopy
                  ? "明文已保存，可重复复制"
                  : "旧版仅有校验值：补录原密钥一次即可复制"}
              </p>
            </div>
            <div className="mg-access-key-actions">
              <ActionButton
                type="button"
                variant="outline"
                size="sm"
                disabled={!key.canCopy || !!copying}
                aria-label={`复制密钥 ${key.name}`}
                onClick={() => void copy(key)}
              >
                <Copy size={13} aria-hidden />
                {copying === key.id ? "复制中…" : "复制"}
              </ActionButton>
              <ActionButton
                type="button"
                variant="outline"
                size="sm"
                disabled={busy}
                aria-label={`重命名密钥 ${key.name}`}
                onClick={() =>
                  setEditor({ kind: "rename", key, revision: current.revision })
                }
              >
                重命名
              </ActionButton>
              <ActionButton
                type="button"
                variant="outline"
                size="sm"
                disabled={busy}
                aria-label={`${key.enabled ? "停用" : "启用"}密钥 ${key.name}`}
                onClick={() =>
                  void change(current.revision, {
                    action: "set_enabled",
                    keyId: key.id,
                    enabled: !key.enabled,
                  }).catch((reason) => setError(errorText(reason)))
                }
              >
                {key.enabled ? "停用" : "启用"}
              </ActionButton>
              <ActionButton
                type="button"
                variant="ghost"
                size="sm"
                disabled={busy}
                aria-label={`删除密钥 ${key.name}`}
                onClick={() =>
                  setEditor({ kind: "remove", key, revision: current.revision })
                }
              >
                删除
              </ActionButton>
              <ActionButton
                type="button"
                variant="ghost"
                size="sm"
                disabled={busy}
                aria-label={`补录原密钥 ${key.name}`}
                onClick={() =>
                  setEditor({
                    kind: "restore_copy",
                    key,
                    revision: current.revision,
                  })
                }
              >
                补录原密钥
              </ActionButton>
            </div>
          </li>
        ))}
      </ul>
      {error && (
        <Alert variant="destructive">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
      <p role="status" className="mg-network-message">
        {message}
      </p>
      {editor && (
        <KeyEditor
          editor={editor}
          busy={busy}
          onClose={() => setEditor(null)}
          onSave={async (operation) => {
            await change(editor.revision, operation);
            setEditor(null);
          }}
        />
      )}
      {manualCopy != null && (
        <Modal
          title="手动复制访问密钥"
          description="剪贴板写入被拒绝。已按你的请求读取这把密钥，请选中复制；关闭后不再显示。"
          onClose={() => setManualCopy(null)}
          footer={
            <ActionButton onClick={() => setManualCopy(null)}>
              完成
            </ActionButton>
          }
        >
          <Field id="manual-access-key-copy" label="要复制的访问密钥">
            <Input
              id="manual-access-key-copy"
              value={manualCopy}
              readOnly
              autoComplete="off"
              spellCheck={false}
              onFocus={(event) => event.currentTarget.select()}
            />
          </Field>
        </Modal>
      )}
    </section>
  );
}

function KeyEditor({
  editor,
  busy,
  onClose,
  onSave,
}: {
  editor: Editor;
  busy: boolean;
  onClose: () => void;
  onSave: (operation: AccessKeyOperation) => Promise<void>;
}) {
  const id = useId();
  const [name, setName] = useState(editor.key?.name ?? "");
  const [value, setValue] = useState("");
  const [showValue, setShowValue] = useState(false);
  const [error, setError] = useState("");
  const [fieldError, setFieldError] = useState<"name" | "value" | null>(null);
  const [saving, setSaving] = useState(false);
  const needsName = editor.kind === "add" || editor.kind === "rename";
  const needsValue = editor.kind === "add" || editor.kind === "restore_copy";
  const title = {
    add: "添加访问密钥",
    rename: "重命名访问密钥",
    restore_copy: "补录原密钥",
    remove: "删除访问密钥",
  }[editor.kind];
  function generate() {
    const bytes = crypto.getRandomValues(new Uint8Array(32));
    setValue(
      `ccm_${Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("")}`,
    );
    setError("");
    setFieldError(null);
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy || saving) return;
    setError("");
    setFieldError(null);
    if (
      needsName &&
      (!name.trim() || [...name.trim()].length > 60 || /\p{Cc}/u.test(name))
    ) {
      setFieldError("name");
      setError("名称需为 1–60 个字符，不能包含控制字符。");
      document.getElementById(`${id}-name`)?.focus();
      return;
    }
    if (needsValue && !/^[\x21-\x7e]{32,256}$/.test(value)) {
      setFieldError("value");
      setError("请填写 32–256 位无空白 ASCII 密钥，或生成随机密钥。");
      document.getElementById(`${id}-value`)?.focus();
      return;
    }
    const operation: AccessKeyOperation =
      editor.kind === "add"
        ? { action: "add", name: name.trim(), value }
        : editor.kind === "rename"
          ? { action: "rename", keyId: editor.key!.id, name: name.trim() }
          : editor.kind === "restore_copy"
            ? { action: "restore_copy", keyId: editor.key!.id, value }
            : { action: "remove", keyId: editor.key!.id };
    setSaving(true);
    try {
      await onSave(operation);
    } catch (reason) {
      setError(errorText(reason));
    } finally {
      setSaving(false);
    }
  }
  return (
    <Modal
      title={title}
      description={
        editor.kind === "remove"
          ? `删除“${name}”后无法再复制，其后续调用会被拒绝。已记录的日志仍保留名称；已开始的响应不主动中断。`
          : editor.kind === "restore_copy"
            ? `旧版没有保留可还原的明文。请补录“${name}”的原密钥，不会更换密钥或改变访问权限。`
            : "密钥只供网关接入，和 Provider 的 API Key 无关。保存后可在列表中重复复制。"
      }
      busy={busy || saving}
      onClose={onClose}
      footer={
        <>
          <ActionButton
            type="button"
            variant="outline"
            disabled={busy || saving}
            onClick={onClose}
          >
            取消
          </ActionButton>
          <ActionButton
            type="submit"
            form={id}
            variant={editor.kind === "remove" ? "destructive" : "default"}
            disabled={busy || saving}
          >
            {editor.kind === "remove" ? "确认删除" : "保存密钥"}
          </ActionButton>
        </>
      }
    >
      <form
        id={id}
        className="mg-form-stack"
        onSubmit={(event) => void submit(event)}
        noValidate
      >
        {needsName && (
          <Field id={`${id}-name`} label="密钥名称">
            <Input
              id={`${id}-name`}
              autoComplete="off"
              value={name}
              disabled={busy || saving}
              onChange={(event) => setName(event.target.value)}
              aria-invalid={fieldError === "name"}
              aria-describedby={error ? `${id}-error` : undefined}
              placeholder="例如：MacBook、CI、同事电脑"
            />
          </Field>
        )}
        {needsValue && (
          <Field
            id={`${id}-value`}
            label={editor.kind === "restore_copy" ? "原密钥" : "密钥内容"}
          >
            <Input
              id={`${id}-value`}
              name="gatewayAccessKey"
              type={showValue ? "text" : "password"}
              autoComplete="new-password"
              spellCheck={false}
              value={value}
              disabled={busy || saving}
              onChange={(event) => setValue(event.target.value)}
              aria-invalid={fieldError === "value"}
              aria-describedby={error ? `${id}-error` : undefined}
            />
            <div className="mg-network-actions">
              {editor.kind === "add" && (
                <ActionButton
                  type="button"
                  variant="outline"
                  disabled={busy || saving}
                  onClick={generate}
                >
                  <KeyRound size={14} aria-hidden />
                  生成随机密钥
                </ActionButton>
              )}
              <ActionButton
                type="button"
                variant="outline"
                aria-pressed={showValue}
                onClick={() => setShowValue(!showValue)}
              >
                {showValue ? "隐藏密钥" : "显示密钥"}
              </ActionButton>
            </div>
          </Field>
        )}
        {error && (
          <p id={`${id}-error`} role="alert" className="mg-network-error">
            {error}
          </p>
        )}
      </form>
    </Modal>
  );
}
