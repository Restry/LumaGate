import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { RefreshCw } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
  Shell,
  type ProviderFeedback,
  type Workspace,
} from "./workspaces";
import { SourceWorkbench } from "./SourceWorkbench";
import { NetworkSettings } from "./NetworkSettings";
import { initialCatalogView } from "./catalog-view";
import { Updates, UpdateBanner } from "./Updates";
import { useUpdateState } from "./use-update";
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
import { CodexLogin } from "./CodexLogin";
import { RequestLogs } from "./RequestLogs";
import { Overview } from "./Overview";
import { HelpPage } from "./Help";
import type { LogFilters } from "./LogStudio";
import {
  displayModel,
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
import "./redesign.css";

type Message = { text: string; kind?: "success" | "error" | "warning" };
const errorText = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

export default function ManualApp({
  initialWorkspace = "sources",
}: { initialWorkspace?: Workspace } = {}) {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [tab, setTab] = useState<Workspace>(initialWorkspace);
  const [agentTab, setAgentTab] = useState<Agent | undefined>(undefined);
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
  const [copilotLogin, setCopilotLogin] = useState(false);
  const [copilotReset, setCopilotReset] = useState(false);
  const [codexLogin, setCodexLogin] = useState(false);
  const [codexReset, setCodexReset] = useState(false);
  const [logNavigation, setLogNavigation] = useState<{
    id: number;
    filters: Partial<LogFilters>;
  } | null>(null);
  const updateView = useUpdateState();
  const [updateDismissed, setUpdateDismissed] = useState(false);
  const [confirmStop, setConfirmStop] = useState(false);
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
    feedback(source.id, "正在刷新模型目录…");
    try {
      const count = await invoke<number>("manual_discover", {
        providerId: source.id,
      });
      feedback(source.id, `已获取 ${count} 个模型`);
      return { text: `已获取 ${count} 个模型` };
    } catch (error) {
      const text = `刷新失败：${errorText(error)}。已有目录和启停选择保持不变`;
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
    setEditor(emptySource());
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
        // 刚保存的来源直接选中，右栏立刻反映它的模型与候选状态。
        setView((current) => ({
          ...current,
          providerId: source.id,
          selectedId: null,
        }));
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
        }}
        busy={busy}
        onGateway={() => {
          if (snapshot?.status.running) setConfirmStop(true);
          else
            void action(async () => {
              await invoke("manual_gateway", { running: true });
              return { text: "网关已启动" };
            });
        }}
        onCopyDirectory={() => {
          if (snapshot)
            void copy(`${snapshot.baseUrl}/v1/models`, "模型目录地址");
        }}
        hasUpdate={!!updateView?.prompt && !updateDismissed}
      >
        <Updates settingsVisible={tab === "settings"} />
        {updateView?.prompt && !updateDismissed && (
          <UpdateBanner
            view={updateView}
            onInstall={() => {
              void invoke("manual_update_install");
              setUpdateDismissed(true);
            }}
            onDismiss={() => setUpdateDismissed(true)}
          />
        )}
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
                onProviders={() => setTab("sources")}
                onAgents={() => setTab("agents")}
              />
            )}
            {tab === "help" && <HelpPage />}
            <div hidden={tab !== "sources"} className="mg-catalog-host">
              <SourceWorkbench
                snapshot={snapshot}
                view={view}
                onViewChange={setView}
                sources={doc!.providers}
                feedback={providerFeedback}
                refreshingId={refreshingId}
                copilotStatus={snapshot.copilotAuth}
                codexStatus={snapshot.codexAuth}
                busy={busy}
                testingKey={testingKey}
                onAdd={addProvider}
                onRefresh={() => void refreshSnapshot()}
                onToggle={(source, enabled) =>
                  void action(async () => {
                    await invoke("manual_set_provider_enabled", {
                      providerId: source.id,
                      enabled,
                      revision: doc!.revision,
                    });
                    return {
                      text: `${source.name} 已${enabled ? "启用" : "停用"}`,
                    };
                  })
                }
                onEdit={(source) => {
                  if (source.codex) setCodexLogin(true);
                  else if (source.copilot) setCopilotLogin(true);
                  else setEditor(structuredClone(source));
                }}
                onDelete={setDeleting}
                onDiscover={(source) =>
                  void action(
                    () => discover(source),
                    `正在拉取 ${source.name} 的模型…`,
                  )
                }
                onCandidate={(modelId, providerId, enabled) =>
                  void action(async () => {
                    await invoke("manual_set_model_enabled", {
                      providerId,
                      modelId,
                      enabled,
                      revision: doc!.revision,
                    });
                    return {
                      text: `${modelId} 已${enabled ? "启用" : "停用"}`,
                    };
                  })
                }
                onBatchCandidates={(providerId, enabled, modelIds) =>
                  void action(async () => {
                    const changed = await invoke<number>(
                      "manual_set_models_enabled_batch",
                      {
                        providerId,
                        modelIds: modelIds ?? null,
                        enabled,
                        revision: doc!.revision,
                      },
                    );
                    return {
                      text: `已${enabled ? "启用" : "停用"} ${changed} 个模型`,
                    };
                  })
                }
                onCostEstimation={(source, enabled) =>
                  void action(async () => {
                    await invoke("manual_set_provider_cost_estimation", {
                      providerId: source.id,
                      enabled,
                      revision: doc!.revision,
                    });
                    return {
                      text: `${source.name} 费用估算已${enabled ? "开启" : "关闭"}`,
                    };
                  })
                }
                onBulkSelect={(source) => setModelEditor(structuredClone(source))}
                onCopilotLogin={() => setCopilotLogin(true)}
                onCopilotReset={() => setCopilotReset(true)}
                onCodexLogin={() => setCodexLogin(true)}
                onCodexReset={() => setCodexReset(true)}
                onOpenAgents={() => setTab("agents")}
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
                        ? `${displayModel(group)} · ${name}：通过（${result.latencyMs} ms）`
                        : `${displayModel(group)} · ${name}：${result.detail}`,
                      kind: result.success ? "success" : "error",
                    };
                  }, `正在测试 ${displayModel(group)}…`);
                }}
                onBlock={(modelIds, blocked) =>
                  void action(async () => {
                    await invoke("manual_set_model_blocked", {
                      modelIds,
                      blocked,
                      revision: doc!.revision,
                    });
                    if (blocked)
                      setView((current) => ({ ...current, selectedId: null }));
                    return {
                      text: `已${blocked ? "停用" : "启用"}模型`,
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
                    return { text: "协议已更新" };
                  })
                }
                onEditSource={(source) => {
                  if (source.codex) setCodexLogin(true);
                  else if (source.copilot) setCopilotLogin(true);
                  else setEditor(structuredClone(source));
                }}
              />
            </div>
            {tab === "agents" && (
              <AgentWorkspace
                agent={agentTab}
                onAgentChange={setAgentTab}
                disabled={busy}
                canSync={activeGroups.length > 0}
                activeGroups={activeGroups}
                agentsStatus={snapshot.agents}
                selectedModel={activeGroups.find(
                  (group) => group.id === view.selectedId,
                )}
                receipts={receipts}
                onModels={() => setTab("sources")}
                onCopy={(text, label) => void copy(text, label)}
                onLaunch={(agent, name) =>
                  void action(async () => {
                    await invoke("manual_launch", { agent });
                    return { text: `已启动 ${name}` };
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
                onSyncNow={(agent) =>
                  void action(async () => {
                    await invoke("manual_sync_now", { agent });
                    return { text: "可用模型目录已同步" };
                  }, "正在同步模型目录…")
                }
                onToggleAutoSync={(agent, enabled) =>
                  void action(async () => {
                    await invoke("manual_set_auto_sync", { agent, enabled });
                    return {
                      text: enabled ? "已开启自动更新" : "已关闭自动更新",
                    };
                  })
                }
                onDisconnectCodex={() =>
                  void action(async () => {
                    const msg = await invoke<string>("manual_disconnect_codex");
                    return { text: msg };
                  }, "正在断开并还原 Codex 配置…")
                }
                onSetCodexDefaultModel={(modelId) =>
                  void action(async () => {
                    await invoke("manual_set_codex_default_model", { modelId });
                    return { text: `已将 ${modelId} 设为 Codex 默认模型` };
                  })
                }
                onMatchCodexModels={(modelIds) =>
                  void action(async () => {
                    const matched = await invoke<number>(
                      "manual_match_codex_models",
                      { modelIds, revision: doc!.revision },
                    );
                    return {
                      text: `已按本机 Codex 列表匹配启用 ${matched} 个模型`,
                    };
                  }, "正在匹配本机 Codex 模型…")
                }
                onBlockModel={(modelIds, blocked) =>
                  void action(async () => {
                    await invoke("manual_set_model_blocked", {
                      modelIds,
                      blocked,
                      revision: doc!.revision,
                    });
                    return { text: blocked ? "已停用模型" : "已启用模型" };
                  })
                }
              />
            )}
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
      {codexLogin && (
        <CodexLogin
          status={snapshot?.codexAuth}
          onClose={() => {
            setCodexLogin(false);
            void refreshSnapshot();
          }}
          onDisconnect={() => setCodexReset(true)}
          onConnected={async (providerId) => {
            setTab("sources");
            setView((current) => ({
              ...current,
              providerId,
              selectedId: null,
            }));
            await refreshSnapshot();
            await action(async () => {
              try {
                const count = await invoke<number>("manual_discover", {
                  providerId,
                });
                feedback(providerId, `已连接 Codex 订阅，获取 ${count} 个模型`);
                return { text: `Codex 订阅已连接，获取 ${count} 个模型` };
              } catch (error) {
                const text = `Codex 订阅已连接，但模型拉取失败：${errorText(error)}`;
                feedback(providerId, text, true);
                return { text, kind: "error" };
              }
            }, "正在拉取 Codex 订阅模型…");
            setCodexLogin(false);
          }}
        />
      )}
      {codexReset && (
        <Modal
          title="断开 Codex 订阅？"
          description="将移除 LumaGate 中的 Codex 订阅来源和缓存凭据，不影响你本机 ~/.codex/auth.json 的登录。"
          busy={busy}
          onClose={() => setCodexReset(false)}
          footer={
            <>
              <ActionButton
                variant="outline"
                disabled={busy}
                onClick={() => setCodexReset(false)}
              >
                取消
              </ActionButton>
              <ActionButton
                disabled={busy}
                onClick={() =>
                  void action(async () => {
                    await invoke("manual_codex_disconnect", {
                      revision: doc!.revision,
                    });
                    setCodexReset(false);
                    setView((c) => ({ ...c, providerId: "all", selectedId: null }));
                    return { text: "Codex 订阅已断开" };
                  })
                }
              >
                确认断开
              </ActionButton>
            </>
          }
        />
      )}
      {copilotLogin && (
        <CopilotLogin
          onClose={() => {
            setCopilotLogin(false);
            void refreshSnapshot();
          }}
          onConnected={async (providerId) => {
            setTab("sources");
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
      {confirmStop && (
        <Modal
          title="停止网关？"
          description="停止后正在生成的请求会被中断，新的请求将无法路由。确定要停止吗？"
          busy={busy}
          onClose={() => setConfirmStop(false)}
          footer={
            <>
              <ActionButton
                variant="outline"
                disabled={busy}
                onClick={() => setConfirmStop(false)}
              >
                取消
              </ActionButton>
              <ActionButton
                disabled={busy}
                onClick={() =>
                  void action(async () => {
                    await invoke("manual_gateway", { running: false });
                    setConfirmStop(false);
                    return { text: "网关已停止" };
                  })
                }
              >
                确认停止
              </ActionButton>
            </>
          }
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
              setView((current) => ({
                ...current,
                providerId: source.id,
                selectedId: null,
              }));
              return warning ?? { text: "候选范围已保存，未测试或同步 Agent" };
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
              setAgentTab(preview.agent);
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
