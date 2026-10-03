import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { ArrowLeft, RefreshCw } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
  Shell,
  ProviderList,
  type ProviderFeedback,
  type Workspace,
} from "./workspaces";
import { ModelCatalog } from "./ModelCatalog";
import { NetworkSettings } from "./NetworkSettings";
import { initialCatalogView } from "./catalog-view";
import { AgentWorkspace, type SyncReceipt } from "./AgentWorkspace";
import {
  ProviderEditor,
  ProviderModelsEditor,
  RoutingEditor,
  MetadataViewer,
  SyncConfirmation,
  DeleteConfirmation,
} from "./dialogs";
import { ActionButton, Modal, Notifications } from "./ui";
import { CopilotLogin } from "./CopilotLogin";
import { RequestLogs } from "./RequestLogs";
import { Overview } from "./Overview";
import { HelpPage } from "./Help";
import type { LogFilters } from "./LogStudio";
import {
  emptySource,
  isBlocked,
  testKey,
  type Agent,
  type Snapshot,
  type Source,
  type Group,
  type Preview,
  type GatewayDocument,
  type KeyUpdate,
  type GatewayNetworkSettings,
  type AccessKeyChangeResult,
  type TestResult,
} from "./types";
import "./manual.css";
import "./catalog.css";
import "./assets/fonts.css";
import "./dashboard-client.css";

type Message = { text: string; kind?: "success" | "error" | "warning" };
const errorText = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

export default function ManualApp({
  initialWorkspace = "models",
}: { initialWorkspace?: Workspace } = {}) {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [tab, setTab] = useState<Workspace>(initialWorkspace);
  const [view, setView] = useState({ ...initialCatalogView });
  const [busy, setBusy] = useState(false);
  const [testingKey, setTestingKey] = useState<string | null>(null);
  const [refreshingId, setRefreshingId] = useState<string | null>(null);
  const pending = useRef(false);
  const mounted = useRef(true);
  const readId = useRef(0);
  const [editor, setEditor] = useState<Source | null>(null);
  const [modelEditor, setModelEditor] = useState<Source | null>(null);
  const [route, setRoute] = useState<Group | null>(null);
  const [metadata, setMetadata] = useState<Group | null>(null);
  const [preview, setPreview] = useState<(Preview & { agent: Agent }) | null>(
    null,
  );
  const [receipts, setReceipts] = useState<Partial<Record<Agent, SyncReceipt>>>(
    {},
  );
  const [deleting, setDeleting] = useState<Source | null>(null);
  const [providerFeedback, setProviderFeedback] = useState<ProviderFeedback>(
    {},
  );
  const [returnToModel, setReturnToModel] = useState(false);
  const [copilotLogin, setCopilotLogin] = useState(false);
  const [copilotReset, setCopilotReset] = useState(false);
  const [logNavigation, setLogNavigation] = useState<{
    id: number;
    filters: Partial<LogFilters>;
  } | null>(null);
  function openLogs(filters: Partial<LogFilters> = {}) {
    setLogNavigation((current) => ({ id: (current?.id ?? 0) + 1, filters }));
    setTab("logs");
  }

  const refreshSnapshot = useCallback(async () => {
    const id = ++readId.current;
    setLoading(true);
    try {
      const next = await invoke<Snapshot>("manual_snapshot");
      if (mounted.current && id === readId.current) {
        setSnapshot(next);
        setLoadError(false);
      }
    } catch {
      if (mounted.current && id === readId.current) setLoadError(true);
    } finally {
      if (mounted.current && id === readId.current) setLoading(false);
    }
  }, []);
  useEffect(() => {
    mounted.current = true;
    void refreshSnapshot();
    return () => {
      mounted.current = false;
      readId.current += 1;
    };
  }, [refreshSnapshot]);

  // Configuration mutations remain serialized. Clipboard and navigation do not acquire this lock.
  async function action(
    task: () => Promise<Message | void>,
    label = "处理中…",
  ) {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setActionError(null);
    const id = toast.loading(label, { id: "manual-action" });
    try {
      const message = await task();
      await refreshSnapshot();
      if (message)
        toast[message.kind ?? "success"](message.text, {
          id,
          duration: message.kind === "error" ? 6000 : 4000,
        });
      else toast.dismiss(id);
    } catch (error) {
      setActionError(errorText(error));
      toast.error(errorText(error), { id, duration: 6000 });
    } finally {
      pending.current = false;
      setBusy(false);
      setTestingKey(null);
      setRefreshingId(null);
    }
  }
  async function copy(text: string, label: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast.success(`${label}已复制`);
    } catch {
      toast.error(`无法复制${label}，请从详情中手动复制。`, { duration: 6000 });
    }
  }
  async function save(
    document: GatewayDocument,
    keyUpdate?: KeyUpdate,
  ): Promise<Message | undefined> {
    const warnings = await invoke<string[] | undefined>("manual_save", {
      document,
      keyUpdate,
    });
    return warnings?.length
      ? { text: warnings.join("\n"), kind: "warning" }
      : undefined;
  }
  function feedback(id: string, text: string, error = false) {
    setProviderFeedback((current) => ({ ...current, [id]: { text, error } }));
  }
  async function discover(source: Source): Promise<Message> {
    setRefreshingId(source.id);
    feedback(source.id, "正在获取模型目录，已有目录仍可查看…");
    try {
      const count = await invoke<number>("manual_discover", {
        providerId: source.id,
      });
      feedback(
        source.id,
        `已获取 ${count} 个模型，保留已有启停选择。未执行推理测试或同步客户端。`,
      );
      return { text: `已拉取 ${count} 个模型` };
    } catch (error) {
      const text = `获取目录失败：${errorText(error)}。已有目录和启停选择保持不变。${source.copilot ? "授权失效时请点击“重新登录”；网络或限流错误可稍后刷新。" : "请检查连接后重试“刷新模型”。"}`;
      feedback(source.id, text, true);
      return { text, kind: "error" };
    }
  }
  const doc = snapshot?.document;
  const groups = snapshot?.groups ?? [];
  const activeGroups = snapshot
    ? groups.filter((group) => !isBlocked(group, snapshot.document))
    : [];
  function addProvider() {
    setTab("providers");
    setReturnToModel(false);
    setEditor(emptySource());
  }
  function viewProvider(source: Source) {
    setView({ ...initialCatalogView, providerId: source.id });
    setTab("models");
  }
  function saveProvider(
    source: Source,
    keyUpdate?: KeyUpdate,
    getModels = false,
  ) {
    void action(
      async () => {
        if (!doc) return;
        const warning = await save(
          {
            ...doc,
            providers: doc.providers.some((item) => item.id === source.id)
              ? doc.providers.map((item) =>
                  item.id === source.id ? source : item,
                )
              : [...doc.providers, source],
          },
          keyUpdate,
        );
        setEditor(null);
        feedback(
          source.id,
          source.models.length
            ? "连接信息已保存，未自动测试或同步 Agent。"
            : "来源已保存。下一步：拉取模型目录。",
        );
        if (getModels) {
          const result = await discover(source);
          return warning && result.kind !== "error"
            ? { text: `${result.text}。${warning.text}`, kind: "warning" }
            : result;
        }
        return warning ?? { text: "Provider 已保存" };
      },
      getModels ? "保存来源并获取模型目录…" : "保存来源…",
    );
  }

  return (
    <>
      <Notifications />
      <Shell
        snapshot={snapshot}
        tab={tab}
        onTab={(next) => {
          setTab(next);
          if (next === "models") setReturnToModel(false);
        }}
        busy={busy}
        onGateway={() =>
          void action(async () => {
            await invoke("manual_gateway", {
              running: !snapshot?.status.running,
            });
            return {
              text: snapshot?.status.running ? "网关已停止" : "网关已启动",
            };
          })
        }
        onCopyDirectory={() => {
          if (snapshot)
            void copy(`${snapshot.baseUrl}/v1/models`, "模型目录地址");
        }}
      >
        {!snapshot ? (
          <section className="mg-backend-state" aria-busy={loading}>
            <h1>{loadError ? "无法读取本机网关状态" : "正在读取本机网关…"}</h1>
            <p>
              {loadError
                ? "尚未读取到配置，并不代表没有 Provider。请重试；若仍失败，请重新打开桌面应用。"
                : "读取已有来源和模型，不会修改配置或执行测试。"}
            </p>
            {loadError && (
              <ActionButton
                disabled={loading}
                onClick={() => void refreshSnapshot()}
              >
                <RefreshCw size={15} aria-hidden />
                重新连接
              </ActionButton>
            )}
          </section>
        ) : (
          <>
            {loadError && (
              <Alert className="mg-workspace-alert">
                <AlertDescription>
                  状态刷新失败，当前显示上次读取的数据。
                  <ActionButton
                    variant="outline"
                    size="sm"
                    disabled={loading || busy}
                    onClick={() => void refreshSnapshot()}
                  >
                    重试读取状态
                  </ActionButton>
                </AlertDescription>
              </Alert>
            )}
            {snapshot.logStorage &&
              !snapshot.logStorage.ready &&
              tab !== "logs" && (
                <Alert variant="destructive" className="mg-workspace-alert">
                  <AlertDescription>
                    日志存储不可用，但配置仍可查看。历史文件未被清空；处理好存储问题前不会启动网关。
                    <ActionButton
                      variant="outline"
                      size="sm"
                      onClick={() => setTab("logs")}
                    >
                      查看日志存储
                    </ActionButton>
                  </AlertDescription>
                </Alert>
              )}
            {actionError && (
              <Alert variant="destructive" className="mg-workspace-alert">
                <AlertDescription>
                  <p>操作未完成：{actionError}</p>
                  <p>请检查提示后重试原操作。配置冲突时先刷新状态。</p>
                  <ActionButton
                    variant="ghost"
                    size="sm"
                    onClick={() => setActionError(null)}
                  >
                    关闭提示
                  </ActionButton>
                </AlertDescription>
              </Alert>
            )}
            {tab === "overview" && (
              <Overview
                snapshot={snapshot}
                onLogs={openLogs}
                onProviders={() => setTab("providers")}
              />
            )}
            {tab === "help" && <HelpPage />}
            <div hidden={tab !== "models"} className="mg-catalog-host">
              <ModelCatalog
                snapshot={snapshot}
                view={view}
                onViewChange={setView}
                busy={busy}
                testingKey={testingKey}
                onAdd={addProvider}
                onProviders={() => setTab("providers")}
                onRefresh={() => void refreshSnapshot()}
                onTest={(group, providerId) => {
                  if (pending.current) return;
                  setTestingKey(testKey(group.id, providerId));
                  const name =
                    group.providerNames[
                      group.providerIds.indexOf(providerId)
                    ] ?? providerId;
                  void action(async () => {
                    const result = await invoke<TestResult>("manual_test", {
                      groupId: group.id,
                      providerId,
                    });
                    return {
                      text: result.success
                        ? `${group.model.id} · ${name}：测试通过 · ${result.latencyMs} ms`
                        : `${group.model.id} · ${name}：${result.detail}`,
                      kind: result.success ? "success" : "error",
                    };
                  }, `正在测试 ${group.model.id} · ${name}…`);
                }}
                onBlock={(modelId, blocked) =>
                  void action(async () => {
                    await invoke("manual_set_model_blocked", {
                      modelId,
                      blocked,
                      revision: doc!.revision,
                    });
                    if (blocked)
                      setView((current) => ({ ...current, selectedId: null }));
                    return {
                      text: `${modelId} 已${blocked ? "屏蔽，可在“已屏蔽”中恢复" : "恢复，可在全部模型中查看"}`,
                    };
                  })
                }
                onRoute={setRoute}
                onMetadata={setMetadata}
                onCopy={(group) =>
                  void copy(group.publicId ?? group.id, "模型 ID")
                }
                onProtocol={(group, protocol) =>
                  void action(async () => {
                    await invoke("manual_set_model_protocol", {
                      groupId: group.id,
                      protocol,
                      revision: doc!.revision,
                    });
                    return { text: "Endpoint 已更新" };
                  })
                }
                onEditSource={(source) => {
                  setTab("providers");
                  setReturnToModel(true);
                  if (source.copilot) setCopilotLogin(true);
                  else setEditor(structuredClone(source));
                }}
                onUse={() => setTab("agents")}
              />
            </div>
            {tab === "providers" && (
              <>
                {returnToModel && (
                  <ActionButton
                    variant="ghost"
                    size="sm"
                    className="mg-return-model"
                    onClick={() => {
                      setTab("models");
                      setReturnToModel(false);
                    }}
                  >
                    <ArrowLeft size={14} aria-hidden />
                    返回刚才的模型
                  </ActionButton>
                )}
                <ProviderList
                  sources={doc!.providers}
                  disabled={busy}
                  feedback={providerFeedback}
                  refreshingId={refreshingId}
                  onToggle={(source, enabled) =>
                    void action(async () => {
                      await invoke("manual_set_provider_enabled", {
                        providerId: source.id,
                        enabled,
                        revision: doc!.revision,
                      });
                      feedback(
                        source.id,
                        enabled
                          ? "来源已启用，恢复参与路由。"
                          : "来源已停用；模型、配置与登录均保留，可随时恢复。",
                      );
                      return {
                        text: `${source.name} 已${enabled ? "启用" : "停用"}`,
                      };
                    })
                  }
                  copilotStatus={snapshot.copilotAuth}
                  onCopilotLogin={() => setCopilotLogin(true)}
                  onCopilotReset={() => setCopilotReset(true)}
                  onAdd={addProvider}
                  onEdit={(source) => setEditor(structuredClone(source))}
                  onManage={(source) => setModelEditor(structuredClone(source))}
                  onView={viewProvider}
                  onDelete={setDeleting}
                  onDiscover={(source) =>
                    void action(
                      () => discover(source),
                      `正在拉取 ${source.name} 的模型…`,
                    )
                  }
                />
              </>
            )}
            <div hidden={tab !== "agents"}>
              <AgentWorkspace
                disabled={busy}
                canSync={activeGroups.length > 0}
                selectedModel={activeGroups.find(
                  (group) => group.id === view.selectedId,
                )}
                receipts={receipts}
                onModels={() => setTab("models")}
                onCopy={(text, label) => void copy(text, label)}
                onLaunch={(agent, name) =>
                  void action(async () => {
                    await invoke("manual_launch", { agent });
                    return { text: `已请求启动 ${name}` };
                  })
                }
                onSync={(agent) =>
                  void action(async () => {
                    const next = await invoke<Preview>("manual_preview_sync", {
                      agent,
                    });
                    setPreview({ ...next, agent });
                  })
                }
              />
            </div>
            <div hidden={tab !== "logs"}>
              <RequestLogs
                active={tab === "logs"}
                navigation={logNavigation}
                gatewayRunning={snapshot.status.running}
                onStorageChange={() => void refreshSnapshot()}
              />
            </div>
            <div hidden={tab !== "settings"}>
              {snapshot.network ? (
                <NetworkSettings
                  network={snapshot.network}
                  busy={busy}
                  onRefresh={() => void refreshSnapshot()}
                  onCopy={(value, label) => void copy(value, label)}
                  onCopyKey={(keyId) =>
                    invoke<void>("manual_copy_access_key", { keyId })
                  }
                  onRevealKey={(keyId) =>
                    invoke<string>("manual_reveal_access_key", { keyId })
                  }
                  onKeyChange={async (revision, operation) => {
                    let result: AccessKeyChangeResult | undefined;
                    let failure: unknown;
                    await action(async () => {
                      try {
                        result = await invoke<AccessKeyChangeResult>(
                          "manual_update_access_key",
                          { revision, operation },
                        );
                      } catch (error) {
                        failure = error;
                        throw error;
                      }
                      return result.warnings.length
                        ? { text: result.warnings.join("；"), kind: "warning" }
                        : { text: "访问密钥变更已生效，无需重启网关" };
                    }, "正在保存访问密钥…");
                    if (failure) throw new Error(errorText(failure));
                    if (!result)
                      throw new Error("另一个操作正在进行，请稍后重试");
                    return result;
                  }}
                  onSave={async (settings) => {
                    let saved: GatewayNetworkSettings | undefined;
                    await action(async () => {
                      saved = await invoke<GatewayNetworkSettings>(
                        "manual_save_gateway_settings",
                        { settings },
                      );
                      setPreview(null);
                      setReceipts({});
                      return {
                        text: "网络设置已保存，未自动重启网关或同步 Agent",
                      };
                    }, "正在保存网络设置…");
                    return saved;
                  }}
                />
              ) : (
                <Alert>
                  <AlertDescription>
                    未读取到网络设置，请刷新状态或重新打开更新后的应用。
                  </AlertDescription>
                </Alert>
              )}
            </div>
          </>
        )}
      </Shell>
      {copilotLogin && (
        <CopilotLogin
          onClose={() => {
            setCopilotLogin(false);
            void refreshSnapshot();
          }}
          onConnected={async (providerId) => {
            setTab("providers");
            await refreshSnapshot();
            await action(async () => {
              try {
                const count = await invoke<number>("manual_discover", {
                  providerId,
                });
                feedback(
                  providerId,
                  `已连接 GitHub 并获取 ${count} 个 Copilot 对话模型。独立路由，未自动测试或同步客户端。`,
                );
                return { text: `Copilot 已登录，获取 ${count} 个模型` };
              } catch (error) {
                const text = `GitHub 登录流程已完成，但模型目录获取失败：${errorText(error)}。已有目录与选择保留，可点击“刷新模型”重试；授权失效时需重新登录。`;
                feedback(providerId, text, true);
                return { text, kind: "error" };
              }
            }, "正在获取 Copilot 模型目录…");
            setCopilotLogin(false);
          }}
        />
      )}
      {copilotReset && (
        <Modal
          title="清理本机 Copilot 登录？"
          description="移除本机凭据及 Copilot 来源，不删除历史日志，也不修改客户端文件。GitHub 上的授权需在 GitHub 设置中另行撤销。"
          busy={busy}
          onClose={() => setCopilotReset(false)}
          footer={
            <>
              <ActionButton
                variant="outline"
                disabled={busy}
                onClick={() => setCopilotReset(false)}
              >
                取消
              </ActionButton>
              <ActionButton
                disabled={busy}
                onClick={() =>
                  void action(async () => {
                    await invoke("manual_copilot_disconnect", {
                      revision: doc!.revision,
                    });
                    setCopilotReset(false);
                    return { text: "本机 Copilot 登录已清理" };
                  })
                }
              >
                确认清理
              </ActionButton>
            </>
          }
        />
      )}
      {editor && (
        <ProviderEditor
          key={editor.id}
          source={editor}
          isNew={!doc?.providers.some((item) => item.id === editor.id)}
          busy={busy}
          onClose={() => setEditor(null)}
          onSave={saveProvider}
        />
      )}
      {modelEditor && (
        <ProviderModelsEditor
          key={modelEditor.id}
          source={modelEditor}
          busy={busy}
          onClose={() => setModelEditor(null)}
          onSave={(source) =>
            void action(async () => {
              if (!doc) return;
              const warning = await save({
                ...doc,
                providers: doc.providers.map((item) =>
                  item.id === source.id
                    ? { ...item, models: source.models }
                    : item,
                ),
              });
              setModelEditor(null);
              viewProvider(source);
              return warning ?? { text: "模型选择已保存，未测试或同步 Agent" };
            })
          }
        />
      )}
      {route && (
        <RoutingEditor
          key={route.id}
          group={route}
          groups={activeGroups}
          busy={busy}
          onClose={() => setRoute(null)}
          onSave={(policy) =>
            void action(async () => {
              if (!doc) return;
              const warning = await save({
                ...doc,
                policies: { ...doc.policies, [route.id]: policy },
              });
              setRoute(null);
              return warning ?? { text: "路由策略已保存" };
            })
          }
        />
      )}
      {metadata && (
        <MetadataViewer
          group={metadata}
          sources={doc?.providers ?? []}
          onClose={() => setMetadata(null)}
        />
      )}
      {preview && (
        <SyncConfirmation
          preview={preview}
          busy={busy}
          onClose={() => setPreview(null)}
          onApply={() =>
            void action(async () => {
              const files = await invoke<SyncReceipt["files"]>(
                "manual_apply_sync",
                { planId: preview.id },
              );
              setReceipts((current) => ({
                ...current,
                [preview.agent]: { files, appliedAt: new Date().toISOString() },
              }));
              setPreview(null);
              return {
                text: `已同步 ${files.length} 个文件，请按客户端说明使用网关`,
              };
            })
          }
        />
      )}
      {deleting && (
        <DeleteConfirmation
          source={deleting}
          busy={busy}
          onClose={() => setDeleting(null)}
          onDelete={() =>
            void action(async () => {
              if (!doc) return;
              if (deleting.copilot) {
                await invoke("manual_copilot_disconnect", {
                  revision: doc.revision,
                });
                setDeleting(null);
                return { text: "Copilot 来源与本机登录已移除，历史日志保留" };
              }
              const warning = await save({
                ...doc,
                providers: doc.providers.filter(
                  (item) => item.id !== deleting.id,
                ),
              });
              setDeleting(null);
              return warning ?? { text: "Provider 已删除" };
            })
          }
        />
      )}
    </>
  );
}
