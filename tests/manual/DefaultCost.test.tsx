import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { DefaultCost, type DefaultCostData } from "@/manual/DefaultCost";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
afterEach(cleanup);
const data: DefaultCostData = {
  totals: { USD: "0.0000001" },
  parts: { USD: ["0", "0.0000001", "0", "0"] },
  covered: 1,
  partial: 0,
  excluded: 1,
  missing: [{ model: "custom-alias", reason: "未定价", requests: 1 }],
  catalog: {
    source: "https://developers.openai.com/api/docs/pricing.md",
    fetchedAt: "2026-10-06T00:00:00Z",
    checkedAt: null,
    error: null,
    models: 40,
    unit: "per 1M tokens",
  },
};
it("keeps tiny nonzero amounts, unknown models and refresh failure visible", async () => {
  vi.mocked(invoke).mockRejectedValue("价格来源不可达，保留上次目录");
  const user = userEvent.setup();
  render(<DefaultCost data={data} onRefresh={() => {}} />);
  expect(screen.getByText("USD <0.000001")).toBeInTheDocument();
  await user.tab();
  await user.keyboard("{Enter}");
  expect(await screen.findByText("精确合计：USD 0.0000001")).toBeVisible();
  await user.click(screen.getByText("查看未计入或部分计价的模型"));
  expect(screen.getByText("custom-alias · 未定价 · 1 条")).toBeVisible();
  await user.click(screen.getByRole("button", { name: "更新默认单价" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("保留上次目录");
  expect(screen.getByText("精确合计：USD 0.0000001")).toBeVisible();
  await user.keyboard("{Escape}");
  expect(
    screen.getByRole("button", { name: "费用口径与价格来源" }),
  ).toHaveFocus();
});
it("does not turn entirely unpriced usage into a zero-dollar bill", () => {
  render(
    <DefaultCost
      data={{ ...data, totals: {}, parts: {}, covered: 0 }}
      onRefresh={() => {}}
    />,
  );
  expect(screen.getByText("未计价")).toBeVisible();
  expect(screen.queryByText(/USD/)).not.toBeInTheDocument();
});
