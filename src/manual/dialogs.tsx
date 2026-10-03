import { useState } from "react";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { ActionButton, Choice, Field, Modal } from "./ui";
import { ProviderModelPicker } from "./ProviderModelPicker";
import {
  compatible,
  keyUpdateFor,
  protocolPaths,
  type Source,
  type KeyUpdate,
  type Protocol,
  type Group,
  type Policy,
  type Preview,
} from "./types";

export function ProviderEditor({
  source,
  isNew,
  busy,
  onClose,
  onSave,
}: {
  source: Source;
  isNew: boolean;
  busy: boolean;
  onClose: () => void;
  onSave: (source: Source, key?: KeyUpdate, discover?: boolean) => void;
}) {
  const [draft, setDraft] = useState(() => structuredClone(source));
  const [secret, setSecret] = useState("");
  const [useEnv, setUseEnv] = useState(!!source.keyEnv && !source.keyRef);
  const [remove, setRemove] = useState(false);
  return (
    <Modal
      title={isNew ? "添加 Provider" : "编辑 Provider"}
      description="填写连接地址与 API Key。"
      busy={busy}
      onClose={onClose}
      footer={
        <>
          <ActionButton variant="outline" disabled={busy} onClick={onClose}>
            取消
          </ActionButton>
          <ActionButton
            form="provider-editor"
            type="submit"
            variant={isNew ? "outline" : "default"}
            disabled={busy}
          >
            保存，不同步
          </ActionButton>
          {isNew && (
            <ActionButton
              form="provider-editor"
              type="submit"
              name="intent"
              value="discover"
              disabled={busy}
            >
              保存并获取模型
            </ActionButton>
          )}
        </>
      }
    >
      <form
        id="provider-editor"
        aria-busy={busy}
        onSubmit={(event) => {
          event.preventDefault();
          onSave(
            { ...draft, keyEnv: useEnv ? draft.keyEnv : "" },
            keyUpdateFor(source, secret, useEnv, remove),
            (event.nativeEvent as SubmitEvent).submitter?.getAttribute(
              "value",
            ) === "discover",
          );
        }}
      >
        <fieldset className="mg-form-grid" disabled={busy}>
          <Field id="provider-name" label="名称">
            <Input
              id="provider-name"
              required
              maxLength={200}
              placeholder="例如：我的模型服务"
              value={draft.name}
              onChange={(event) =>
                setDraft({ ...draft, name: event.target.value })
              }
            />
          </Field>
          <Field id="provider-url" label="API 地址">
            <Input
              id="provider-url"
              type="url"
              required
              placeholder="https://api.example.com/v1"
              value={draft.baseUrl}
              onChange={(event) =>
                setDraft({ ...draft, baseUrl: event.target.value })
              }
            />
          </Field>
          {useEnv ? (
            <Field
              wide
              id="provider-key-env"
              label="环境变量名称"
              hint="也可关闭下方高级选项，直接粘贴 API Key。"
            >
              <Input
                id="provider-key-env"
                value={draft.keyEnv}
                placeholder="OPENAI_API_KEY"
                onChange={(event) =>
                  setDraft({ ...draft, keyEnv: event.target.value })
                }
              />
            </Field>
          ) : (
            <Field
              wide
              id="provider-api-key"
              label="API Key"
              hint="明文保存在 ~/.lumagate/keys/，不使用钥匙串，不同步给 Agent。无需密钥的本地服务可留空。"
            >
              <Input
                id="provider-api-key"
                type="password"
                autoComplete="new-password"
                maxLength={8192}
                disabled={remove}
                placeholder={
                  source.keyRef ? "已保存，留空保留原密钥" : "直接粘贴 API Key"
                }
                value={secret}
                onChange={(event) => setSecret(event.target.value)}
              />
              {source.keyRef && (
                <div className="mg-check-row">
                  <Checkbox
                    id="remove-saved-key"
                    checked={remove}
                    onCheckedChange={(value) => {
                      setRemove(value === true);
                      if (value === true) setSecret("");
                    }}
                  />
                  <Label htmlFor="remove-saved-key">
                    移除已保存密钥（保存后生效）
                  </Label>
                </div>
              )}
            </Field>
          )}
          <details className="mg-advanced" open={useEnv}>
            <summary>高级选项</summary>
            <div className="mg-check-row">
              <Checkbox
                id="use-env-key"
                checked={useEnv}
                onCheckedChange={(value) => setUseEnv(value === true)}
              />
              <Label htmlFor="use-env-key">
                使用环境变量，替代直接填写密钥
              </Label>
            </div>
            <Field
              id="directory-auth"
              label="目录鉴权方式"
              hint="只影响 /v1/models 的目录请求；模型调用的 Endpoint 在模型列表中设置。"
            >
              <Choice
                id="directory-auth"
                value={
                  draft.protocol === "anthropic" ? "anthropic" : "openai_chat"
                }
                onChange={(value) =>
                  setDraft({ ...draft, protocol: value as Protocol })
                }
                options={[
                  { value: "openai_chat", label: "Bearer（通用）" },
                  { value: "anthropic", label: "Anthropic x-api-key" },
                ]}
              />
            </Field>
          </details>
          <div className="mg-check-row mg-field--wide">
            <Checkbox
              id="provider-enabled"
              checked={draft.enabled}
              onCheckedChange={(value) =>
                setDraft({ ...draft, enabled: value === true })
              }
            />
            <Label htmlFor="provider-enabled">启用 Provider</Label>
          </div>
        </fieldset>
      </form>
    </Modal>
  );
}

export function ProviderModelsEditor({
  source,
  busy,
  onClose,
  onSave,
}: {
  source: Source;
  busy: boolean;
  onClose: () => void;
  onSave: (source: Source) => void;
}) {
  const [models, setModels] = useState(() => structuredClone(source.models));
  return (
    <Modal
      title={`选择模型 · ${source.name}`}
      description="选择本网关可使用的模型。保存前只修改草稿，不会自动测试或同步 Agent。"
      busy={busy}
      onClose={onClose}
      footer={
        <>
          <ActionButton variant="outline" disabled={busy} onClick={onClose}>
            取消
          </ActionButton>
          <ActionButton
            disabled={busy}
            onClick={() => onSave({ ...source, models })}
          >
            保存模型选择
          </ActionButton>
        </>
      }
    >
      <ProviderModelPicker
        models={models}
        disabled={busy}
        onChange={setModels}
      />
    </Modal>
  );
}

export function RoutingEditor({
  group,
  groups,
  busy,
  onClose,
  onSave,
}: {
  group: Group;
  groups: Group[];
  busy: boolean;
  onClose: () => void;
  onSave: (policy: Policy) => void;
}) {
  const [policy, setPolicy] = useState<Policy>(() => ({
    ...structuredClone(group.policy),
    fallbacks: group.policy.fallbacks.map(
      (id) =>
        groups.find((g) => g.id === id || g.legacyIds?.includes(id))?.id ?? id,
    ),
  }));
  const choices = groups.filter(
    (g) =>
      !g.model.id.startsWith("copilot/") &&
      !group.model.id.startsWith("copilot/") &&
      compatible(group, g),
  );
  return (
    <Modal
      title="路由设置"
      description={`${group.model.id} · ${protocolPaths[group.protocol]}`}
      busy={busy}
      onClose={onClose}
      footer={
        <>
          <ActionButton variant="outline" disabled={busy} onClick={onClose}>
            取消
          </ActionButton>
          <ActionButton disabled={busy} onClick={() => onSave(policy)}>
            保存策略
          </ActionButton>
        </>
      }
    >
      <div className="mg-form-stack">
        <Field id="balance" label="负载均衡">
          <Choice
            id="balance"
            value={policy.balance}
            onChange={(balance) =>
              setPolicy({ ...policy, balance: balance as Policy["balance"] })
            }
            options={[
              { value: "round_robin", label: "轮询：分摊到同组部署" },
              { value: "priority", label: "优先顺序：优先使用第一部署" },
            ]}
          />
        </Field>
        <div className="mg-check-row">
          <Checkbox
            id="failover"
            checked={policy.failover}
            onCheckedChange={(value) =>
              setPolicy({ ...policy, failover: value === true })
            }
          />
          <Label htmlFor="failover">失败时切换下一部署</Label>
        </div>
        <section className="mg-fallbacks">
          <div className="mg-section-heading">
            <h3>后备模型</h3>
            {policy.fallbacks.length > 0 && (
              <ActionButton
                size="sm"
                variant="ghost"
                onClick={() => setPolicy({ ...policy, fallbacks: [] })}
              >
                清空后备
              </ActionButton>
            )}
          </div>
          <p className="mg-hint">
            只列出 Endpoint 与能力一致的模型。按点击顺序尝试，最多 3 个。
          </p>
          {choices.map((g) => (
            <label className="mg-fallback-option" key={g.id}>
              <Checkbox
                checked={policy.fallbacks.includes(g.id)}
                disabled={
                  busy ||
                  (!policy.fallbacks.includes(g.id) &&
                    policy.fallbacks.length >= 3)
                }
                onCheckedChange={(value) =>
                  setPolicy({
                    ...policy,
                    fallbacks:
                      value === true
                        ? [...policy.fallbacks, g.id]
                        : policy.fallbacks.filter((id) => id !== g.id),
                  })
                }
              />
              <span>{g.model.id}</span>
              {policy.fallbacks.includes(g.id) && (
                <small>优先 {policy.fallbacks.indexOf(g.id) + 1}</small>
              )}
            </label>
          ))}
          {!choices.length && (
            <p className="mg-empty-inline">暂无兼容的后备模型。</p>
          )}
        </section>
      </div>
    </Modal>
  );
}

export function MetadataViewer({
  group,
  sources,
  onClose,
}: {
  group: Group;
  sources: Source[];
  onClose: () => void;
}) {
  return (
    <Modal
      title="模型元数据"
      description={`${group.model.id}：各 Provider 的 /v1/models 原始条目。仅查看，不发模型请求。`}
      wide
      onClose={onClose}
      footer={
        <ActionButton variant="outline" onClick={onClose}>
          关闭
        </ActionButton>
      }
    >
      {group.providerIds.map((id) => {
        const source = sources.find((p) => p.id === id);
        const model = source?.models.find((m) => m.id === group.model.id);
        return (
          <section className="mg-code-section" key={id}>
            <h3>{source?.name}</h3>
            <pre>
              {JSON.stringify(
                model?.metadata ?? { status: "尚未从 /v1/models 拉取元数据" },
                null,
                2,
              )}
            </pre>
          </section>
        );
      })}
    </Modal>
  );
}

export function SyncConfirmation({
  preview,
  busy,
  onClose,
  onApply,
}: {
  preview: Preview;
  busy: boolean;
  onClose: () => void;
  onApply: () => void;
}) {
  return (
    <Modal
      title="确认同步配置"
      description="这是唯一会写入 Agent 配置的操作。仅修改受管字段，先备份，再检查并发变化。"
      wide
      busy={busy}
      onClose={onClose}
      footer={
        <>
          <ActionButton variant="outline" disabled={busy} onClick={onClose}>
            取消，不写入
          </ActionButton>
          <ActionButton disabled={busy} onClick={onApply}>
            确认写入配置
          </ActionButton>
        </>
      }
    >
      <p className="mg-sync-note">{preview.note}</p>
      {preview.files.map((file) => (
        <section className="mg-code-section" key={file.path}>
          <h3>{file.path}</h3>
          <p className="mg-hint">
            {file.beforeHash
              ? `修改现有文件 · 校验 ${file.beforeHash.slice(0, 12)}`
              : "新建文件"}
          </p>
          <p className="mg-hint">受管字段的写入摘要（不是完整文件差异）</p>
          <pre>{JSON.stringify(file.changes, null, 2)}</pre>
        </section>
      ))}
    </Modal>
  );
}

export function DeleteConfirmation({
  source,
  busy,
  onClose,
  onDelete,
}: {
  source: Source;
  busy: boolean;
  onClose: () => void;
  onDelete: () => void;
}) {
  return (
    <Modal
      title="删除 Provider"
      description={
        source.copilot
          ? "移除 Copilot 来源与本机登录，保留历史日志，不修改客户端配置。GitHub 网站上的授权不会自动撤销，如需彻底撤销，请到 GitHub 设置中操作。"
          : `删除 ${source.name}、模型部署及保存的密钥。不会修改已经同步到 Agent 的配置。`
      }
      busy={busy}
      onClose={onClose}
      footer={
        <>
          <ActionButton variant="outline" disabled={busy} onClick={onClose}>
            取消
          </ActionButton>
          <ActionButton
            variant="destructive"
            disabled={busy}
            onClick={onDelete}
          >
            确认删除
          </ActionButton>
        </>
      }
    />
  );
}
