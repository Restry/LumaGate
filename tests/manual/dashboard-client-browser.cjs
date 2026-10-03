const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const fs = require("node:fs");
const path = require("node:path");
const assert = require("node:assert/strict");
const origin = process.env.PREVIEW_ORIGIN || "http://127.0.0.1:4178";
const out = path.resolve(".artifacts/manual/dashboard-client");
fs.mkdirSync(out, { recursive: true });
(async () => {
  const b = await chromium.launch({ channel: "chrome", headless: true });
  const p = await b.newPage({
    viewport: { width: 1440, height: 1000 },
    reducedMotion: "reduce",
  });
  p.setDefaultTimeout(10000);
  const report = {
    scope:
      "Production React components with isolated mock IPC; no real backend, files, credentials or model calls",
    passed: [],
    errors: [],
    external: [],
    layouts: [],
    accessibility: [],
  };
  p.on("pageerror", (e) => report.errors.push(e.message));
  p.on("request", (r) => {
    if (!r.url().startsWith(origin)) report.external.push(r.url());
  });
  const nav = (name) => p.getByRole("button", { name, exact: true }).click();
  const step = async (name, work) => {
    await work();
    report.passed.push(name);
    console.log("PASS", name);
  };
  const scan = async (name, width, theme) => {
    if (!process.env.AXE_MODULE) return;
    await p.addScriptTag({ path: require.resolve(process.env.AXE_MODULE) });
    const result = await p.evaluate(
      async () =>
        await axe.run(document, {
          runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21aa"] },
        }),
    );
    report.accessibility.push({
      name,
      width,
      theme,
      violations: result.violations.map((v) => ({
        id: v.id,
        impact: v.impact,
        nodes: v.nodes.map((n) => ({
          target: n.target,
          summary: n.failureSummary,
        })),
      })),
    });
  };
  try {
    await p.goto(origin + "/tests/manual/preview.html?dashboard=1", {
      waitUntil: "networkidle",
    });
    await p.waitForFunction(
      () =>
        document
          .querySelector(".mc-metrics [data-number-text]")
          ?.getAttribute("data-number-text") !== "—",
    );
    await step(
      "Default collapsed sidebar, keyboard toggle and genuine overview data flow",
      async () => {
        assert.equal(
          await p
            .locator(".mg-sidebar")
            .evaluate((e) => e.getBoundingClientRect().width),
          68,
        );
        await p.getByRole("button", { name: "展开侧栏", exact: true }).focus();
        await p.keyboard.press("Enter");
        assert.equal(
          await p
            .locator(".mg-sidebar")
            .evaluate((e) => e.getBoundingClientRect().width),
          218,
        );
        await p.keyboard.press("Space");
        assert.equal(
          await p
            .locator(".mg-sidebar")
            .evaluate((e) => e.getBoundingClientRect().width),
          68,
        );
        assert.equal(await p.locator(".mc-metrics article").count(), 4);
        assert(await p.locator(".mc-chart svg").isVisible());
        const calls = await p.evaluate(() => window.__fixtureCalls);
        assert(
          calls.every((c) =>
            ["manual_snapshot", "manual_query_logs"].includes(c),
          ),
        );
        await p.getByRole("button", { name: "Token", exact: true }).click();
        assert(
          await p
            .getByRole("heading", { name: "用量趋势", exact: true })
            .isVisible(),
        );
        await p.getByRole("button", { name: "请求", exact: true }).click();
      },
    );
    await step(
      "Overview exceptions prefilter the real log workspace; compact rows and inspector keep HTTP/usage diagnostics",
      async () => {
        const failures = Number(
          await p.locator(".mc-secondary-grid .mc-count").innerText(),
        );
        await p.getByRole("button", { name: "查看日志", exact: true }).click();
        await p.getByRole("table", { name: "网关调用日志" }).waitFor();
        await p.waitForFunction(
          (expected) =>
            document.querySelectorAll(".mg-log-table tbody tr").length ===
            expected,
          failures,
        );
        assert(
          (await p.getByLabel("当前筛选全部记录统计").innerText()).includes(
            "失败",
          ),
        );
        const height = await p
          .locator(".mg-log-table tbody tr")
          .first()
          .evaluate((e) => e.getBoundingClientRect().height);
        assert(height <= 44, `row height ${height}`);
        await p
          .getByRole("button", { name: /展开请求/ })
          .first()
          .focus();
        await p.keyboard.press("Enter");
        await p
          .getByRole("button", { name: "关闭请求详情", exact: true })
          .waitFor();
        assert(
          (await p.getByText("Provider 尝试顺序", { exact: true }).count()) ===
            0,
        );
        assert(await p.getByLabel("Provider 尝试顺序").isVisible());
        await p.screenshot({ path: path.join(out, "log-inspector.png") });
        await p
          .getByRole("button", { name: "关闭请求详情", exact: true })
          .click();
      },
    );
    await step(
      "Provider search and model selection preserve the explicit save boundary",
      async () => {
        await nav("Provider");
        await p
          .getByRole("textbox", { name: "搜索 Provider", exact: true })
          .fill("Forge");
        assert.equal(await p.locator(".mc-provider-choice").count(), 1);
        await p
          .getByRole("button", { name: "选择来源 Forge API", exact: true })
          .click();
        await p
          .getByRole("button", { name: "选择 Forge API 的模型", exact: true })
          .click();
        const dialog = p.getByRole("dialog");
        await dialog.waitFor();
        assert((await dialog.innerText()).includes("保存前只修改草稿"));
        await dialog.getByRole("button", { name: "取消", exact: true }).click();
        assert(
          !(await p.evaluate(() => window.__fixtureCalls)).includes(
            "manual_save",
          ),
        );
        await p
          .getByRole("textbox", { name: "搜索 Provider", exact: true })
          .fill("");
        await p
          .getByRole("button", { name: "选择来源 North Lab", exact: true })
          .click();
      },
    );
    await step(
      "Model detail reveals source-specific test controls without automatically testing",
      async () => {
        await nav("模型");
        const count = (await p.evaluate(() => window.__fixtureCalls)).filter(
          (c) => c === "manual_test",
        ).length;
        await p
          .getByRole("button", {
            name: "查看模型 FW-Kimi-K2.7-Code",
            exact: true,
          })
          .click();
        await p.getByRole("region", { name: "模型详情" }).waitFor();
        assert(
          (await p
            .getByRole("button", { name: /测试 FW-Kimi-K2.7-Code/ })
            .count()) > 0,
        );
        assert.equal(
          (await p.evaluate(() => window.__fixtureCalls)).filter(
            (c) => c === "manual_test",
          ).length,
          count,
        );
        await p.screenshot({ path: path.join(out, "model-detail.png") });
        await p.getByRole("button", { name: "返回列表", exact: true }).click();
      },
    );
    await step(
      "Client sync preview/cancel never applies configuration or launches a client",
      async () => {
        await nav("客户端");
        await p.getByRole("tab", { name: "Codex", exact: true }).click();
        assert(
          await p
            .getByText("codex --profile cc_switch_manual", { exact: true })
            .isVisible(),
        );
        await p.getByRole("button", { name: "预览同步", exact: true }).click();
        await p.getByRole("dialog").waitFor();
        await p.screenshot({ path: path.join(out, "sync-preview.png") });
        await p
          .getByRole("dialog")
          .getByRole("button", { name: "取消，不写入", exact: true })
          .click();
        const calls = await p.evaluate(() => window.__fixtureCalls);
        assert(calls.includes("manual_preview_sync"));
        assert(!calls.includes("manual_apply_sync"));
        assert(!calls.includes("manual_launch"));
      },
    );
    // Hide completed toast messages and move the pointer before stable captures.
    for (const [id, name] of [
      ["overview", "概览"],
      ["logs", "调用日志"],
      ["providers", "Provider"],
      ["models", "模型"],
      ["agents", "客户端"],
      ["settings", "设置"],
      ["help", "帮助"],
    ]) {
      await p.setViewportSize({ width: 1440, height: 1000 });
      await nav(name);
      await p.waitForTimeout(250);
      await p.mouse.move(1420, 15);
      await p.screenshot({ path: path.join(out, `${id}-light.png`) });
      await scan(id, 1440, "light");
      for (const width of [1440, 1200, 900, 600, 320]) {
        await p.setViewportSize({ width, height: 900 });
        await p.waitForTimeout(70);
        const layout = await p.evaluate(() => ({
          pageOverflow: document.documentElement.scrollWidth > innerWidth,
          workspaceOverflow:
            document.querySelector("main").scrollWidth >
            document.querySelector("main").clientWidth,
        }));
        report.layouts.push({ id, width, ...layout });
        assert.equal(
          layout.pageOverflow,
          false,
          `${id} page overflow at ${width}`,
        );
        assert.equal(
          layout.workspaceOverflow,
          false,
          `${id} main overflow at ${width}`,
        );
      }
      await p.setViewportSize({ width: 1200, height: 800 });
      await p.getByRole("button", { name: "切换外观", exact: true }).click();
      await p.screenshot({ path: path.join(out, `${id}-dark.png`) });
      await scan(id, 1200, "dark");
      await p.getByRole("button", { name: "切换外观", exact: true }).click();
    }
    report.checkedAt = new Date().toISOString();
    report.commands = [
      ...new Set(await p.evaluate(() => window.__fixtureCalls)),
    ];
    fs.writeFileSync(
      path.join(out, "checks.json"),
      JSON.stringify(report, null, 2),
    );
    assert.deepEqual(report.errors, []);
    assert.deepEqual(report.external, []);
    assert(
      report.accessibility.every((scan) => !scan.violations.length),
      "axe findings in checks.json",
    );
    console.log(
      "PASS",
      report.layouts.length,
      "layouts;",
      report.accessibility.length,
      "a11y scans",
    );
  } catch (error) {
    await p.screenshot({ path: path.join(out, "failure.png") }).catch(() => {});
    fs.writeFileSync(
      path.join(out, "failure.json"),
      JSON.stringify({ error: error.message, report }, null, 2),
    );
    throw error;
  } finally {
    await b.close();
  }
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
