import {
  declaredInput,
  displayModel,
  groupModelIds,
  isBlocked,
  stripNamespace,
  testKey,
  type Group,
  type Model,
  type Snapshot,
  type Source,
  type TestResult,
} from "./types";

export type CatalogStatus = "all" | "enabled" | "disabled" | "failed";
export interface CatalogView {
  query: string;
  protocol: string;
  providerId: string;
  status: CatalogStatus;
  selectedId: string | null;
}
export const initialCatalogView: CatalogView = {
  query: "",
  protocol: "all",
  providerId: "all",
  status: "all",
  selectedId: null,
};

export function sourceResults(group: Group, tests: Record<string, TestResult>) {
  const results = group.providerIds.map((id) => tests[testKey(group.id, id)]);
  return {
    passed: results.filter((result) => result?.success === true).length,
    failed: results.filter((result) => result?.success === false).length,
    untested: results.filter((result) => !result).length,
  };
}
export function resultLabel(group: Group, tests: Record<string, TestResult>) {
  const { passed, failed, untested } = sourceResults(group, tests);
  if (failed > 0) return `${failed} 失败`;
  if (passed > 0 && untested === 0) return "已通过";
  if (passed > 0) return `${passed}/${passed + untested} 通过`;
  return "未测试";
}
export function capabilitySummary(model: Model) {
  const facts: string[] = [];
  if (model.contextWindow != null)
    facts.push(`${model.contextWindow.toLocaleString()} 上下文`);
  if (declaredInput(model)?.includes("image")) facts.push("图片");
  if (model.reasoning === true) facts.push("思考");
  if (model.tools === true) facts.push("工具");
  return facts.join(" · ") || "未知";
}

export interface ScopedModelRow {
  model: Model;
  displayName: string;
  group?: Group;
  protocol: string;
}

export function scopedSourceModels(
  source: Source,
  snapshot: Snapshot,
  view: CatalogView,
) {
  const needle = view.query.trim().toLocaleLowerCase();
  const rows: ScopedModelRow[] = source.models.map((model) => {
    const group = snapshot.groups.find(
      (g) =>
        g.providerIds.includes(source.id) &&
        groupModelIds(g).includes(model.id),
    );
    return {
      model,
      displayName: stripNamespace(model.id),
      group,
      protocol: model.protocolOverride ?? group?.protocol ?? source.protocol,
    };
  });
  const counts = {
    all: rows.length,
    enabled: rows.filter((r) => r.model.enabled).length,
    disabled: rows.filter((r) => !r.model.enabled).length,
    failed: rows.filter(
      (r) =>
        r.group &&
        (!!r.group.routingError ||
          sourceResults(r.group, snapshot.document.tests).failed > 0),
    ).length,
  };
  const matches = rows.filter((row) => {
    if (
      needle &&
      !`${row.displayName} ${row.model.id} ${row.model.name ?? ""}`
        .toLocaleLowerCase()
        .includes(needle)
    ) {
      return false;
    }
    if (view.protocol !== "all" && row.protocol !== view.protocol) {
      return false;
    }
    if (view.status === "enabled") return row.model.enabled;
    if (view.status === "disabled") return !row.model.enabled;
    if (view.status === "failed") {
      return (
        !!row.group &&
        (!!row.group.routingError ||
          sourceResults(row.group, snapshot.document.tests).failed > 0)
      );
    }
    return true;
  });
  return { rows, counts, matches };
}

export function catalogEntries(snapshot: Snapshot, view: CatalogView) {
  const { document, groups } = snapshot;
  const active = groups.filter((group) => !isBlocked(group, document));
  const blockedGroups = groups.filter((group) => isBlocked(group, document));
  const blockedIds = [
    ...new Set([
      ...(document.blockedModels ?? []),
      ...blockedGroups.flatMap((group) => groupModelIds(group)),
    ]),
  ];
  const needle = view.query.trim().toLocaleLowerCase();
  const counts = {
    all: active.length,
    enabled: active.length,
    disabled: blockedIds.length,
    failed: active.filter(
      (group) =>
        !!group.routingError || sourceResults(group, document.tests).failed > 0,
    ).length,
  };
  const matches = active.filter((group) => {
    if (
      !`${displayModel(group)} ${groupModelIds(group).join(" ")} ${group.model.name ?? ""} ${group.id}`
        .toLocaleLowerCase()
        .includes(needle)
    )
      return false;
    if (view.protocol !== "all" && group.protocol !== view.protocol)
      return false;
    if (
      view.providerId !== "all" &&
      !group.providerIds.includes(view.providerId)
    )
      return false;
    const result = sourceResults(group, document.tests);
    return view.status === "failed"
      ? !!group.routingError || result.failed > 0
      : true;
  });
  return {
    active,
    counts,
    matches,
    blocked: blockedIds
      .filter((id) => id.toLocaleLowerCase().includes(needle))
      .sort(),
  };
}

export function checkedAtLabel(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? value : date.toLocaleString();
}
