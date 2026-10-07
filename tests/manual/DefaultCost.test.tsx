import { cleanup, render, screen, within } from "@testing-library/react";
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
  estimated: 1,
  assumptions: { model: 1, input: 1 },
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
it("discloses precise estimates and missing models without replacing cached amounts on refresh failure", async () => {
  vi.mocked(invoke).mockRejectedValue("价格来源不可达，保留上次目录");
  const user = userEvent.setup();
  render(<DefaultCost data={data} onRefresh={() => {}} />);
  expect(screen.getByLabelText("预估费用")).toHaveTextContent("USD <0.000001");
  expect(screen.queryByText(/custom-alias/)).not.toBeInTheDocument();
  await user.tab();
  await user.keyboard("{Enter}");
  const detail = await screen.findByRole("dialog", { name: "费用明细" });
  expect(detail).toHaveTextContent("USD 0.0000001");
  expect(detail).toHaveTextContent("未记录的缓存拆分按普通输入价估算");
  await user.click(within(detail).getByText("未计入记录"));
  expect(within(detail).getByText(/custom-alias/)).toBeVisible();
  await user.click(within(detail).getByRole("button", { name: "更新价格" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("保留上次目录");
  expect(detail).toHaveTextContent("USD 0.0000001");
  await user.keyboard("{Escape}");
  expect(screen.getByRole("button", { name: "费用明细" })).toHaveFocus();
});
it("distinguishes unpriced use from explicitly free use", () => {
  const view = render(
    <DefaultCost
      data={{ ...data, totals: {}, parts: {}, covered: 0 }}
      onRefresh={() => {}}
    />,
  );
  expect(screen.getByLabelText("预估费用")).toHaveTextContent("未计价");
  view.rerender(
    <DefaultCost
      data={{ ...data, totals: { USD: "0" }, excluded: 0, missing: [] }}
      onRefresh={() => {}}
    />,
  );
  expect(screen.getByLabelText("预估费用")).toHaveTextContent("USD 0.00");
  expect(screen.queryByText("未计价")).not.toBeInTheDocument();
});
