import { useEffect, useRef, useState } from "react";
import {
  ArrowRight,
  KeyRound,
  Plus,
  RefreshCw,
  Search,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { ActionButton } from "./ui";
import { protocolLabels, type Source } from "./types";
export { DashboardShell as Shell } from "./DashboardShell";
export type Workspace =
  | "overview"
  | "models"
  | "providers"
  | "agents"
  | "logs"
  | "settings"
  | "help";
export type ProviderFeedback = Record<string, { text: string; error: boolean }>;

export function ProviderList({
  sources,
  disabled,
  feedback,
  refreshingId,
  onToggle,
  copilotStatus,
  onCopilotLogin,
  onCopilotReset,
  onAdd,
  onEdit,
  onManage,
  onView,
  onDelete,
  onDiscover,
}: {
  sources: Source[];
  disabled: boolean;
  refreshingId?: string | null;
  onToggle: (source: Source, enabled: boolean) => void;
  feedback: ProviderFeedback;
  copilotStatus?: {
    connected: boolean;
    login: string | null;
    message: string | null;
  };
  onCopilotLogin?: () => void;
  onCopilotReset?: () => void;
  onAdd: () => void;
  onEdit: (source: Source) => void;
  onManage: (source: Source) => void;
  onView: (source: Source) => void;
  onDelete: (source: Source) => void;
  onDiscover: (source: Source) => void;
}) {
  const [selectedId, setSelectedId] = useState(sources[0]?.id ?? ""),
    [search, setSearch] = useState(""),
    [tab, setTab] = useState("connection");
  const knownSources = useRef(new Set(sources.map((p) => p.id)));
  useEffect(() => {
    const added = sources.find((p) => !knownSources.current.has(p.id));
    knownSources.current = new Set(sources.map((p) => p.id));
    if (added) {
      setSelectedId(added.id);
      setTab("connection");
    } else if (!sources.some((p) => p.id === selectedId))
      setSelectedId(sources[0]?.id ?? "");
  }, [sources, selectedId]);
  const selected = sources.find((p) => p.id === selectedId),
    shown = sources.filter((p) =>
      `${p.name} ${p.baseUrl}`.toLowerCase().includes(search.toLowerCase()),
    );
  const notice = selected && feedback[selected.id];
  return (
    <section className="mg-providers-workspace">
      <div className="mg-page-heading">
        <div>
          <h1>
            Provider <span className="mc-count">{sources.length}</span>
          </h1>
        </div>
        <div className="mg-heading-actions">
          {onCopilotLogin && (
            <ActionButton
              variant="outline"
              disabled={disabled}
              onClick={onCopilotLogin}
            >
              <KeyRound size={15} />
              GitHub Copilot
            </ActionButton>
          )}
          <ActionButton disabled={disabled} onClick={onAdd}>
            <Plus size={15} aria-hidden />
            添加 Provider
          </ActionButton>
        </div>
      </div>
      {copilotStatus?.message && (
        <div className="mc-notice" role="alert">
          {copilotStatus.message}
          {(onCopilotLogin || onCopilotReset) && (
            <ActionButton
              variant="outline"
              size="sm"
              disabled={disabled}
              onClick={copilotStatus.login ? onCopilotLogin : onCopilotReset}
            >
              {copilotStatus.login
                ? "重新登录 GitHub"
                : "处理本机 Copilot 登录"}
            </ActionButton>
          )}
        </div>
      )}
      {!sources.length ? (
        <div className="mg-catalog-empty">
          <h2>还没有模型来源</h2>
          <p>添加支持 /v1/models 的服务。获取目录不会自动测试模型。</p>
          <ActionButton disabled={disabled} onClick={onAdd}>
            添加第一个来源
          </ActionButton>
        </div>
      ) : (
        <div className="mc-provider-layout">
          <div className="mc-provider-list">
            <div className="mg-search">
              <Search size={15} aria-hidden />
              <Input
                aria-label="搜索 Provider"
                placeholder="搜索 Provider…"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
            <p className="mc-provider-list-label">
              已配置 <span>{sources.length}</span>
            </p>
            {shown.map((p) => (
              <button
                className="mc-provider-choice"
                key={p.id}
                aria-label={`选择来源 ${p.name}`}
                aria-pressed={p.id === selectedId}
                onClick={() => {
                  setSelectedId(p.id);
                  setTab("connection");
                }}
              >
                <span className="mc-avatar">{p.name.slice(0, 1)}</span>
                <span>
                  <strong>{p.name}</strong>
                  <small>
                    {p.models.length} 个模型 · {p.enabled ? "已启用" : "已停用"}
                  </small>
                </span>
              </button>
            ))}
            {!shown.length && <p className="mc-empty">没有匹配来源</p>}
            <p className="mc-provider-note">
              <ShieldCheck size={14} />
              密钥与连接独立管理，不同步给客户端。
            </p>
          </div>
          {selected && (
            <section
              className="mc-provider-panel"
              aria-label={`来源 ${selected.name}`}
            >
              <header className="mc-provider-heading">
                <span className="mc-avatar">{selected.name.slice(0, 1)}</span>
                <div>
                  <h2>{selected.name}</h2>
                  <p>{selected.baseUrl}</p>
                </div>
                <label className="mc-source-status">
                  <span>{selected.enabled ? "已启用" : "已停用"}</span>
                  <Switch
                    aria-label={`启用来源 ${selected.name}`}
                    checked={selected.enabled}
                    disabled={disabled}
                    onCheckedChange={(enabled) => onToggle(selected, enabled)}
                  />
                </label>
              </header>
              <div className="mc-provider-toolbar">
                <p>
                  {selected.enabled
                    ? "参与路由；停用后仍保留模型与登录。"
                    : "已暂停路由，模型、配置与登录均保留。"}
                </p>
                <ActionButton
                  variant="outline"
                  size="sm"
                  disabled={disabled}
                  aria-busy={refreshingId === selected.id}
                  onClick={() => onDiscover(selected)}
                >
                  <RefreshCw size={14} aria-hidden />
                  {refreshingId === selected.id ? "正在刷新模型…" : "刷新模型"}
                </ActionButton>
              </div>
              <div className="mc-section-tabs" aria-label="Provider 详情">
                <button
                  aria-pressed={tab === "connection"}
                  onClick={() => setTab("connection")}
                >
                  连接设置
                </button>
                <button
                  aria-pressed={tab === "models"}
                  onClick={() => setTab("models")}
                >
                  来源模型 <span>{selected.models.length}</span>
                </button>
              </div>
              <div className="mc-provider-body">
                {tab === "connection" ? (
                  <>
                    <dl className="mc-connection-fields">
                      <div>
                        <dt>名称</dt>
                        <dd>{selected.name}</dd>
                      </div>
                      <div>
                        <dt>目录鉴权</dt>
                        <dd>
                          {selected.copilot
                            ? "GitHub 设备授权 · 个人账号"
                            : selected.protocol === "anthropic"
                              ? "Anthropic x-api-key"
                              : "Bearer（通用）"}
                        </dd>
                      </div>
                      <div className="mc-field-wide">
                        <dt>Base URL</dt>
                        <dd>
                          <code>{selected.baseUrl}</code>
                        </dd>
                      </div>
                      <div className="mc-field-wide">
                        <dt>{selected.copilot ? "GitHub 账号" : "API 密钥"}</dt>
                        <dd>
                          {selected.copilot
                            ? copilotStatus?.connected
                              ? `${copilotStatus.login} · 已保存登录`
                              : "需要重新登录 GitHub"
                            : selected.keyRef
                              ? "API Key 已保存 · 编辑留空保留原密钥"
                              : selected.keyEnv
                                ? `使用环境变量 ${selected.keyEnv}`
                                : "未配置密钥"}
                        </dd>
                        <dd className="mc-field-note">
                          {selected.copilot
                            ? "Copilot 会话按需刷新；凭据保存在本机私有目录，不回显、不写入客户端。模型使用 copilot/ 独立路由，目录存在不代表已经测试可用。"
                            : "明文保存在本机私有目录，界面不回显；不写入 Agent 配置。"}
                        </dd>
                      </div>
                    </dl>
                    <div className="mc-provider-operation">
                      <div>
                        <strong>模型目录</strong>
                        <p>
                          {selected.models.length
                            ? `${selected.models.filter((m) => m.enabled).length} / ${selected.models.length} 个模型已启用`
                            : "下一步：拉取模型目录，再选择启用范围。"}
                        </p>
                        <p>获取目录不是推理测试，不自动产生模型调用。</p>
                      </div>
                    </div>
                  </>
                ) : (
                  <>
                    <div className="mc-provider-operation">
                      <span>
                        {selected.models.filter((m) => m.enabled).length} /{" "}
                        {selected.models.length} 个模型已启用
                      </span>
                      <ActionButton
                        variant="outline"
                        size="sm"
                        disabled={disabled || !selected.models.length}
                        aria-label={`选择 ${selected.name} 的模型`}
                        onClick={() => onManage(selected)}
                      >
                        选择模型
                      </ActionButton>
                    </div>
                    <div className="mg-table-wrap">
                      <table
                        className="mc-source-model-table"
                        aria-label={`${selected.name} 的来源模型`}
                      >
                        <thead>
                          <tr>
                            <th>模型</th>
                            <th>接口</th>
                            <th>状态</th>
                          </tr>
                        </thead>
                        <tbody>
                          {selected.models.map((m) => (
                            <tr key={m.id}>
                              <td>{m.id}</td>
                              <td>
                                {
                                  protocolLabels[
                                    m.protocolOverride ?? selected.protocol
                                  ]
                                }
                              </td>
                              <td>{m.enabled ? "已启用" : "已停用"}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                    {!selected.models.length && (
                      <div className="mc-empty">尚未获取模型目录</div>
                    )}
                  </>
                )}
                {notice && (
                  <p
                    className={`mg-source-feedback ${notice.error ? "is-failed" : ""}`}
                    role={notice.error ? "alert" : "status"}
                  >
                    {notice.text}
                  </p>
                )}
              </div>
              <footer className="mc-provider-footer">
                <ActionButton
                  variant="ghost"
                  size="sm"
                  disabled={disabled}
                  className="mg-source-delete"
                  onClick={() => onDelete(selected)}
                  aria-label={`删除来源 ${selected.name}`}
                >
                  <Trash2 size={15} />
                </ActionButton>
                <ActionButton
                  variant="ghost"
                  size="sm"
                  disabled={
                    !selected.models.some((m) => m.enabled) || !selected.enabled
                  }
                  onClick={() => onView(selected)}
                >
                  查看模型
                  <ArrowRight size={14} />
                </ActionButton>
                {tab === "connection" && (
                  <ActionButton
                    variant="outline"
                    size="sm"
                    disabled={disabled || !selected.models.length}
                    aria-label={`选择 ${selected.name} 的模型`}
                    onClick={() => onManage(selected)}
                  >
                    选择模型
                  </ActionButton>
                )}
                <ActionButton
                  variant={selected.copilot ? "outline" : "default"}
                  size="sm"
                  disabled={disabled}
                  onClick={() =>
                    selected.copilot ? onCopilotLogin?.() : onEdit(selected)
                  }
                  aria-label={
                    selected.copilot
                      ? "重新登录 GitHub Copilot"
                      : `编辑来源 ${selected.name}`
                  }
                >
                  {selected.copilot ? "重新登录" : "编辑连接"}
                </ActionButton>
              </footer>
            </section>
          )}
        </div>
      )}
    </section>
  );
}
