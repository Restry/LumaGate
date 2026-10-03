import {
  declaredInput,
  isBlocked,
  testKey,
  type Group,
  type Model,
  type Snapshot,
  type TestResult,
} from "./types";

export type CatalogStatus = "all" | "failed" | "untested" | "blocked";
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
  return (
    [
      passed ? `${passed} 通过` : "",
      failed ? `${failed} 失败` : "",
      untested ? `${untested} 未测试` : "",
    ]
      .filter(Boolean)
      .join(" · ") || "无来源"
  );
}
export function capabilitySummary(model: Model) {
  const facts: string[] = [];
  if (model.contextWindow != null)
    facts.push(`${model.contextWindow.toLocaleString()} 上下文`);
  if (declaredInput(model)?.includes("image")) facts.push("图片输入");
  if (model.reasoning === true) facts.push("思考");
  if (model.tools === true) facts.push("工具调用");
  return facts.join(" · ") || "上游未提供能力信息";
}
export function catalogEntries(snapshot: Snapshot, view: CatalogView) {
  const { document, groups } = snapshot;
  const active = groups.filter((group) => !isBlocked(group, document));
  const blockedIds = [
    ...new Set([
      ...(document.blockedModels ?? []),
      ...groups
        .filter((group) => isBlocked(group, document))
        .map((group) => group.model.id),
    ]),
  ];
  const needle = view.query.trim().toLocaleLowerCase();
  const counts = {
    all: active.length,
    failed: active.filter(
      (group) =>
        !!group.routingError || sourceResults(group, document.tests).failed > 0,
    ).length,
    untested: active.filter(
      (group) => sourceResults(group, document.tests).untested > 0,
    ).length,
    blocked: blockedIds.length,
  };
  const matches = active.filter((group) => {
    if (
      !`${group.model.id} ${group.model.name ?? ""} ${group.id}`
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
      : view.status === "untested"
        ? result.untested > 0
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
