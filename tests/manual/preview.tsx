// Development-only browser fixture. No HTTP requests, real keys, config files or native processes.
import ReactDOM from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { historyFixture, readyStorage } from "./history-fixture";
import type { RequestLog } from "@/manual/RequestLogs";
import ManualApp from "../../src/manual/ManualApp";
import {
  testKey,
  type Group,
  type GatewayDocument,
  type Snapshot,
  type Source,
  type TestResult,
} from "../../src/manual/types";
import "../../src/index.css";

const sources: Source[] = [
  "North Lab",
  "Example Cloud",
  "Forge API",
  "Local Studio",
].map((name, index) => ({
  id: `fixture-${index}`,
  name,
  baseUrl: `https://source-${index}.example/v1`,
  keyEnv: "",
  keyRef: "fixture-reference-not-a-secret",
  enabled: true,
  protocol: "openai_chat",
  models: [],
}));
const names = [
  "FW-Kimi-K2.7-Code",
  "FW-Kimi-K3",
  "Kimi-K2.7-Code",
  "gpt-5.5",
  "claude-sonnet-4-6",
  "gemini-3.1-pro",
  "qwen3-coder",
  "deepseek-v3.2",
  "llama-4-scout",
  "mistral-large",
  "gpt-5.4-mini",
  "gemini-3-flash",
];
const snapshot: Snapshot = {
  document: {
    revision: 1,
    providers: sources,
    policies: {},
    tests: {},
    blockedModels: [],
  },
  groups: [],
  status: { running: true },
  baseUrl: "http://127.0.0.1:15722",
  network: {
    saved: {
      revision: 0,
      listenAddress: "127.0.0.1",
      listenPort: 15722,
      allowLan: false,
    },
    active: {
      revision: 0,
      listenAddress: "127.0.0.1",
      listenPort: 15722,
      allowLan: false,
    },
    accessKeys: {
      revision: 0,
      items: [
        {
          id: "fixture-macbook",
          name: "MacBook",
          enabled: true,
          canCopy: true,
          createdAt: null,
        },
        {
          id: "fixture-ci",
          name: "CI",
          enabled: true,
          canCopy: true,
          createdAt: null,
        },
        {
          id: "legacy",
          name: "旧版密钥",
          enabled: true,
          canCopy: false,
          createdAt: null,
        },
      ],
    },
    restartRequired: false,
    lanAddresses: ["http://192.168.1.10:15722/v1", "http://10.8.0.2:15722/v1"],
  },
};
function updateFixtureAddresses() {
  const network = snapshot.network!;
  const effective = network.active ?? network.saved;
  snapshot.baseUrl = `http://127.0.0.1:${effective.listenPort}`;
  network.lanAddresses = ["192.168.1.10", "10.8.0.2"].map(
    (ip) => `http://${ip}:${effective.listenPort}/v1`,
  );
}
for (let index = 0; index < 89; index++) {
  const id =
    index < names.length
      ? names[index]
      : `example-model-${String(index + 1).padStart(2, "0")}`;
  const providerIds = (
    index < 34 ? [index % 4, (index + 1) % 4] : [index % 4]
  ).map((value) => sources[value].id);
  const model = {
    id,
    enabled: true,
    image: index % 5 === 0,
    tools: index % 3 === 0 ? true : null,
    contextWindow: index < 3 ? null : index % 4 === 0 ? 200000 : 128000,
    reasoning: index % 4 === 0 ? true : null,
    maxOutputTokens: index < 3 ? null : 32768,
  };
  const group: Group = {
    id: `${id}--fixture`,
    model,
    providerIds,
    providerNames: providerIds.map(
      (pid) => sources.find((source) => source.id === pid)!.name,
    ),
    protocol: index % 4 === 3 ? "openai_responses" : "openai_chat",
    protocolMode: "auto",
    policy: { balance: "round_robin", failover: true, fallbacks: [] },
    blocked: index >= 58,
  };
  // The fixture explicitly models the backend's effective limits; source rows stay untouched.
  if (index < 3) {
    group.model = {
      ...model,
      contextWindow: 1_000_000,
      maxOutputTokens: 128_000,
      limitsSource: "user_override",
    };
  }
  snapshot.groups.push(group);
  for (const pid of providerIds)
    sources
      .find((source) => source.id === pid)!
      .models.push(structuredClone(model));
  if (index >= 58) snapshot.document.blockedModels!.push(id);
  if (index < 18)
    providerIds.forEach((pid, number) => {
      const success = index !== 0 || number === 0;
      snapshot.document.tests[testKey(group.id, pid)] = {
        success,
        latencyMs: 820 + index * 103 + number * 600,
        checkedAt: "2026-09-14T04:00:00Z",
        detail: success
          ? "隔离样例：单次文本请求通过。"
          : "隔离样例：HTTP 404 · DeploymentNotFound。该来源未找到对应部署，请核对模型名称与部署路径。",
        providerId: pid,
      };
    });
}
const params = new URLSearchParams(location.search);
if (params.has("dark")) document.documentElement.classList.add("dark");
if (params.has("analysis"))
  window.localStorage.setItem("cc-switch-manual.logs.analysis", "true");
if (params.get("view") === "logs")
  window.localStorage.setItem("cc-switch-manual.logs.analysis", "false");
if (params.has("empty")) {
  snapshot.document.providers = [];
  snapshot.groups = [];
  snapshot.document.blockedModels = [];
}
if (params.has("long")) {
  snapshot.groups[0].model.id = "组织专用-" + "very-long-model-".repeat(9);
  snapshot.groups[0].id = "long-fixture";
}
// Fake credentials exist only in this isolated fixture, never in production or persistent storage.
const fixtureSecrets = new Map([
  ["fixture-macbook", "fixture-macbook-key-0123456789abcdef0123456789"],
  ["fixture-ci", "fixture-ci-key-0123456789abcdef0123456789"],
  ["legacy", "fixture-legacy-key-0123456789abcdef0123456789"],
]);
const fixtureLogs = [
  {
    id: 6,
    endpoint: "/v1/responses",
    status: 200,
    caller: { kind: "key", keyId: "fixture-macbook", name: "MacBook" },
    model: "gpt-5.5",
  },
  {
    id: 5,
    endpoint: "/v1/chat/completions",
    status: 200,
    caller: { kind: "key", keyId: "fixture-ci", name: "CI" },
    model: "qwen3-coder",
  },
  {
    id: 4,
    endpoint: "/v1/responses",
    status: 429,
    caller: { kind: "key", keyId: "fixture-ci", name: "CI" },
    model: "gpt-5.5",
  },
  {
    id: 3,
    endpoint: "/v1/models",
    status: 200,
    caller: { kind: "key", keyId: "fixture-macbook", name: "MacBook" },
    model: null,
  },
  {
    id: 2,
    endpoint: "/v1/messages",
    status: 200,
    caller: { kind: "local" },
    model: "claude-sonnet-4-6",
  },
  {
    id: 1,
    endpoint: "/v1/models",
    status: 401,
    caller: { kind: "rejected" },
    model: null,
  },
].map((row) => ({
  ...row,
  startedAt: `2026-09-15T02:0${row.id}:00Z`,
  responseMs: 120 + row.id * 10,
  streaming: false,
  providers: row.model
    ? [{ id: "fixture-provider", name: "North Lab", outcome: "隔离样例" }]
    : [],
  usage: [6, 5, 2].includes(row.id)
    ? {
        state: "reported",
        inputTokens: row.id === 6 ? 100000 : row.id === 5 ? 12500 : 5200,
        outputTokens: row.id === 6 ? 8000 : row.id === 5 ? 1100 : 400,
        totalTokens: row.id === 6 ? 108000 : row.id === 5 ? 13600 : 5600,
        cacheReadTokens: row.id === 6 ? 25000 : row.id === 2 ? 5000 : 0,
        cacheWriteTokens: row.id === 2 ? 50 : null,
        reasoningTokens: row.id === 6 ? 2000 : null,
        reason: null,
      }
    : null,
  response: { note: "隔离日志样例，没有真实请求" },
  responseState: "已结束",
  input: null,
}));
const calls: string[] = [];
Object.assign(window, { __fixtureCalls: calls });
let logStorageUnavailable = params.has("logerror");
if (logStorageUnavailable) snapshot.status.running = false;
const historyRows: RequestLog[] = params.has("dashboard")
  ? Array.from({ length: 168 }, (_, i) => ({
      ...(structuredClone(fixtureLogs[i % fixtureLogs.length]) as RequestLog),
      id: 2500 - i,
      startedAt: new Date(Date.now() - i * 60 * 60000).toISOString(),
      providers: fixtureLogs[i % fixtureLogs.length].providers.length
        ? [
            {
              id: sources[i % sources.length].id,
              name: sources[i % sources.length].name,
              outcome: "隔离样例",
            },
          ]
        : [],
    }))
  : params.has("history")
    ? (Array.from({ length: 251 }, (_, i) => ({
        ...fixtureLogs[0],
        id: i,
        model: i % 2 ? "Kimi-K3" : "gpt-6-astra",
        startedAt: `2026-09-${i % 2 ? "19" : "20"}T12:00:00Z`,
        status: i % 5 ? 200 : 429,
        usage: i % 5 ? fixtureLogs[0].usage : null,
      })) as RequestLog[])
    : (fixtureLogs as RequestLog[]);
let copilotFlow = 0;
mockIPC(async (command, raw) => {
  calls.push(command);
  const args = (raw ?? {}) as Record<string, any>;
  if (command === "manual_snapshot") {
    if (params.has("offline")) throw new Error("fixture offline");
    return structuredClone({
      ...snapshot,
      logStorage: {
        ...readyStorage,
        ready: !logStorageUnavailable,
        message: logStorageUnavailable
          ? "隔离演示：日志库不可用，历史未删除"
          : readyStorage.message,
      },
    });
  }
  if (command === "manual_copilot_start") {
    copilotFlow += 1;
    return {
      flowId: `fixture-flow-${copilotFlow}`,
      userCode: "DEMO-CODE",
      verificationUri: "https://github.com/login/device",
      expiresIn: params.get("copilot") === "expired" ? 2 : 900,
      interval: 5,
    };
  }
  if (command === "manual_copilot_cancel") {
    copilotFlow += 1;
    return;
  }
  if (command === "manual_copilot_open_login") return;
  if (command === "manual_copilot_poll") {
    if (args.flowId !== `fixture-flow-${copilotFlow}`)
      throw new Error("授权已取消（隔离演示）");
    if (params.get("copilot") === "denied")
      throw new Error("Copilot 权限不足（隔离演示）");
    if (params.get("copilot") === "pending")
      return { state: "pending", interval: 5 };
    if (!snapshot.document.providers.some((p) => p.copilot))
      snapshot.document.providers.push({
        id: "copilot-personal",
        name: "GitHub Copilot",
        baseUrl: "https://api.githubcopilot.com",
        keyEnv: "",
        protocol: "openai_chat",
        enabled: true,
        copilot: { accountId: "123", grantId: "fixture-grant" },
        models: [],
      });
    snapshot.copilotAuth = {
      connected: true,
      login: "fixture-user",
      message: null,
    };
    snapshot.document.revision += 1;
    return {
      state: "connected",
      providerId: "copilot-personal",
      login: "fixture-user",
    };
  }
  if (command === "manual_copilot_disconnect") {
    snapshot.document.providers = snapshot.document.providers.filter(
      (p) => !p.copilot,
    );
    snapshot.groups = snapshot.groups.filter(
      (g) => !g.model.id.startsWith("copilot/"),
    );
    snapshot.copilotAuth = { connected: false, login: null, message: null };
    snapshot.document.revision += 1;
    return;
  }
  if (command === "manual_request_logs") return structuredClone(fixtureLogs);
  if (command === "manual_query_logs")
    return logStorageUnavailable
      ? {
          storage: {
            ...readyStorage,
            ready: false,
            message: "隔离演示：日志库不可用，历史文件未被删除",
          },
        }
      : historyFixture(historyRows, args.query);
  if (command === "manual_retry_log_storage") {
    logStorageUnavailable = false;
    return readyStorage;
  }
  if (command === "manual_open_log_directory") return;
  if (
    command === "manual_reveal_access_key" ||
    command === "manual_copy_access_key"
  ) {
    const key = snapshot.network!.accessKeys.items.find(
      (key) => key.id === args.keyId,
    );
    if (!key?.canCopy)
      throw new Error("请先补录原密钥，旧版校验值不能还原明文");
    const value = fixtureSecrets.get(key.id);
    if (!value) throw new Error("隔离密钥文件缺失");
    if (command === "manual_copy_access_key") {
      await navigator.clipboard.writeText(value);
      return;
    }
    return value;
  }
  if (command === "manual_update_access_key") {
    const keys = snapshot.network!.accessKeys;
    if (args.revision !== keys.revision)
      throw new Error("访问密钥列表已变化，请刷新后重试");
    const op = args.operation;
    if (op.action === "add") {
      if (keys.items.some((key) => key.name === op.name))
        throw new Error("密钥名称不能重复");
      const id = `fixture-created-${keys.revision}`;
      keys.items.push({
        id,
        name: op.name,
        enabled: true,
        canCopy: true,
        createdAt: null,
      });
      fixtureSecrets.set(id, op.value);
    } else {
      const key = keys.items.find((key) => key.id === op.keyId);
      if (!key) throw new Error("访问密钥不存在");
      if (op.action === "rename") key.name = op.name;
      if (op.action === "set_enabled") key.enabled = op.enabled;
      if (op.action === "remove") {
        keys.items = keys.items.filter((key) => key.id !== op.keyId);
        fixtureSecrets.delete(op.keyId);
      }
      if (op.action === "restore_copy") {
        if (op.value !== fixtureSecrets.get(op.keyId))
          throw new Error("与原密钥不匹配；未修改原密钥或访问权限");
        key.canCopy = true;
      }
    }
    keys.revision += 1;
    return { keys: structuredClone(keys), warnings: [] };
  }
  if (command === "manual_test") {
    await new Promise((resolve) => setTimeout(resolve, 650));
    const result: TestResult = {
      success: true,
      latencyMs: 910,
      checkedAt: new Date().toISOString(),
      detail: "隔离样例：仅更新前端测试记录，没有真实请求。",
      providerId: args.providerId,
    };
    snapshot.document.tests[testKey(args.groupId, args.providerId)] = result;
    return result;
  }
  if (command === "manual_save_gateway_settings") {
    const network = snapshot.network!;
    if (args.settings.revision !== network.saved.revision)
      throw new Error("网关设置已变化，请刷新设置后重新保存");
    network.saved = {
      revision: network.saved.revision + 1,
      listenAddress: args.settings.listenAddress,
      listenPort: args.settings.listenPort,
      allowLan: args.settings.listenAddress === "0.0.0.0",
    };
    network.restartRequired =
      !!network.active &&
      (network.active.listenAddress !== network.saved.listenAddress ||
        network.active.listenPort !== network.saved.listenPort);
    updateFixtureAddresses();
    return structuredClone(network.saved);
  }
  if (command === "manual_gateway") {
    snapshot.status.running = args.running;
    snapshot.network!.active = args.running
      ? structuredClone(snapshot.network!.saved)
      : null;
    snapshot.network!.restartRequired = false;
    updateFixtureAddresses();
    return;
  }
  if (command === "manual_set_model_blocked") {
    for (const group of snapshot.groups)
      if (group.model.id === args.modelId) group.blocked = args.blocked;
    snapshot.document.blockedModels = args.blocked
      ? [...snapshot.document.blockedModels!, args.modelId]
      : snapshot.document.blockedModels!.filter((id) => id !== args.modelId);
    snapshot.document.revision += 1;
    return;
  }
  if (command === "manual_set_model_protocol") {
    const group = snapshot.groups.find((entry) => entry.id === args.groupId)!;
    group.protocolMode = args.protocol ? "manual" : "auto";
    group.protocol = args.protocol ?? "openai_chat";
    snapshot.document.revision += 1;
    return;
  }
  if (command === "manual_set_provider_enabled") {
    const source = snapshot.document.providers.find(
      (p) => p.id === args.providerId,
    );
    if (!source || args.revision !== snapshot.document.revision)
      throw new Error("来源已变化（隔离演示）");
    source.enabled = args.enabled;
    snapshot.document.revision++;
    return;
  }
  if (command === "manual_save") {
    snapshot.document = structuredClone(args.document as GatewayDocument);
    snapshot.document.revision += 1;
    return [];
  }
  if (command === "manual_discover") {
    const source = snapshot.document.providers.find(
      (p) => p.id === args.providerId,
    );
    if (source?.copilot) {
      if (params.get("copilot") === "catalog-error")
        throw new Error("目录暂不可用（隔离演示）");
      source.models = ["gpt-fixture", "claude-fixture"].map((id, i) => ({
        id: `copilot/${id}`,
        name: id,
        protocolOverride: i === 0 ? "openai_responses" : "anthropic",
        contextWindow: 128000,
        maxOutputTokens: 16000,
        image: false,
        tools: true,
        enabled:
          source.models.find((model) => model.id === `copilot/${id}`)
            ?.enabled ?? true,
        metadata: {
          id,
          supported_endpoints: i === 0 ? ["/responses"] : ["/v1/messages"],
        },
      }));
      snapshot.groups = snapshot.groups.filter(
        (g) => !g.model.id.startsWith("copilot/"),
      );
      source.models.forEach((model) =>
        snapshot.groups.push({
          id: `${model.id}--fixture`,
          publicId: model.id.replace(/^copilot\//, ""),
          model,
          protocol: model.protocolOverride!,
          protocolMode: "manual",
          providerIds: [source.id],
          providerNames: [source.name],
          policy: { balance: "round_robin", failover: true, fallbacks: [] },
        }),
      );
      snapshot.document.revision += 1;
    }
    return source?.models.length ?? 0;
  }
  if (command === "manual_launch") return;
  if (command === "manual_preview_sync")
    return {
      id: "fixture-plan",
      revision: snapshot.document.revision,
      note: "隔离预览：不写入真实配置，保留默认模型。",
      files: [
        {
          path: `/isolated/${args.agent}/models.json`,
          beforeHash: "fixture-hash",
          changes: {
            managedProvider: "cc_switch_manual",
            defaultModel: "保持不变",
          },
        },
      ],
    };
  if (command === "manual_apply_sync")
    return [
      { path: "/isolated/models.json", backup: "/isolated/models.json.backup" },
    ];
  throw new Error(`Blocked unknown fixture command: ${command}`);
});
ReactDOM.createRoot(document.getElementById("root")!).render(
  <>
    <ManualApp
      initialWorkspace={
        params.has("dashboard")
          ? "overview"
          : params.has("logs")
            ? "logs"
            : "models"
      }
    />
    <span
      style={{
        position: "fixed",
        top: 6,
        left: 16,
        color: "var(--mg-muted)",
        fontSize: 11,
        pointerEvents: "none",
      }}
    >
      枢光 · LumaGate · 隔离演示数据
    </span>
    {params.has("logs") && (
      <button
        type="button"
        onClick={() =>
          historyRows.push({
            ...(structuredClone(fixtureLogs[0]) as RequestLog),
            id: Math.max(0, ...historyRows.map((row) => row.id)) + 1,
            startedAt: new Date().toISOString(),
          })
        }
        style={{
          position: "fixed",
          bottom: 12,
          right: 18,
          border: "1px solid var(--mg-control)",
          borderRadius: 6,
          background: "var(--mg-surface)",
          color: "var(--mg-muted)",
          padding: "6px 10px",
          fontSize: 11,
        }}
      >
        模拟更新 · 不调用模型
      </button>
    )}
  </>,
);
