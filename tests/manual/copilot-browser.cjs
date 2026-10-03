const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const fs = require("node:fs"),
  path = require("node:path"),
  assert = require("node:assert/strict");
const origin = process.env.PREVIEW_ORIGIN || "http://127.0.0.1:4178";
const out = path.resolve(".artifacts/manual/copilot");
fs.mkdirSync(out, { recursive: true });
(async () => {
  const browser = await chromium.launch({ channel: "chrome", headless: true });
  const report = {
    scope:
      "Isolated mock IPC only; no real GitHub login, token, browser authorization or inference",
    flows: [],
    errors: [],
    external: [],
    layouts: [],
    accessibility: [],
  };
  try {
    for (const mode of [
      "pending",
      "success",
      "denied",
      "expired",
      "catalog-error",
    ]) {
      const p = await browser.newPage({
        viewport: { width: 1200, height: 900 },
        reducedMotion: "reduce",
      });
      p.setDefaultTimeout(15000);
      p.on("pageerror", (e) => report.errors.push(e.message));
      p.on("request", (r) => {
        if (!r.url().startsWith(origin)) report.external.push(r.url());
      });
      await p.addInitScript(() => {
        Object.defineProperty(navigator, "clipboard", {
          configurable: true,
          value: {
            writeText: async (text) => {
              window.__copiedFixture = text;
            },
          },
        });
      });
      await p.goto(
        `${origin}/tests/manual/preview.html?dashboard=1&copilot=${mode}`,
        { waitUntil: "networkidle" },
      );
      assert((await p.title()).includes("LumaGate"));
      await p.getByRole("button", { name: "Provider", exact: true }).click();
      assert(
        !(await p.evaluate(() => window.__fixtureCalls)).some((c) =>
          c.startsWith("manual_copilot"),
        ),
      );
      await p
        .getByRole("button", { name: "GitHub Copilot", exact: true })
        .click();
      const dialog = p.getByRole("dialog", { name: "连接 GitHub Copilot" });
      await p.getByLabel("GitHub 设备授权码").waitFor();
      assert.equal(
        await dialog
          .getByRole("button", { name: "登录并获取模型", exact: true })
          .count(),
        0,
      );
      assert.equal(
        (await p.evaluate(() => window.__fixtureCalls)).filter(
          (c) => c === "manual_copilot_start",
        ).length,
        1,
      );
      if (mode === "pending")
        await p.screenshot({ path: path.join(out, "copilot-compact.png") });
      assert.equal(
        await p.locator('input[type="password"]:visible').count(),
        0,
      );
      if (mode === "pending") {
        await p
          .getByRole("button", { name: "复制代码并打开 GitHub", exact: true })
          .click();
        assert.equal(
          await p.evaluate(() => window.__copiedFixture),
          "DEMO-CODE",
        );
        assert(
          (await p.evaluate(() => window.__fixtureCalls)).includes(
            "manual_copilot_open_login",
          ),
        );
        for (const width of [1200, 900, 375, 320]) {
          await p.setViewportSize({ width, height: 900 });
          await p.waitForFunction(() => {
            const r = document
              .querySelector('[role="dialog"]')
              .getBoundingClientRect();
            return r.left >= 0 && r.right <= innerWidth;
          });
          const dims = await dialog.evaluate((el) => ({
            scroll: el.scrollWidth,
            client: el.clientWidth,
            left: el.getBoundingClientRect().left,
            right: el.getBoundingClientRect().right,
          }));
          report.layouts.push({ width, ...dims });
          assert(
            dims.scroll <= dims.client + 1 &&
              dims.left >= 0 &&
              dims.right <= width,
          );
          await p.screenshot({
            path: path.join(out, `device-code-${width}.png`),
          });
        }
        await p.setViewportSize({ width: 1200, height: 900 });
        if (process.env.AXE_MODULE) {
          await p.addScriptTag({
            path: require.resolve(process.env.AXE_MODULE),
          });
          const scan = await p.evaluate(
            async () =>
              await axe.run(document, {
                runOnly: {
                  type: "tag",
                  values: ["wcag2a", "wcag2aa", "wcag21aa"],
                },
              }),
          );
          report.accessibility.push({
            scene: "device-code",
            violations: scan.violations.map((v) => ({
              id: v.id,
              targets: v.nodes.map((n) => n.target),
              details: v.nodes.map((n) => n.failureSummary),
            })),
          });
        }
        await p.keyboard.press("Escape");
        await dialog.waitFor({ state: "hidden" });
        assert(
          (await p.evaluate(() => window.__fixtureCalls)).includes(
            "manual_copilot_cancel",
          ),
        );
        report.flows.push(
          "entry automatically generates one device code; one click copies and opens GitHub; cancellation and responsive dialog",
        );
      } else if (mode === "success") {
        await dialog.waitFor({ state: "hidden" });
        const card = p.getByRole("region", { name: "来源 GitHub Copilot" });
        await card.waitFor();
        assert((await card.innerText()).includes("fixture-user"));
        await p.screenshot({ path: path.join(out, "connected-provider.png") });
        await p.getByRole("button", { name: "模型", exact: true }).click();
        await p
          .getByRole("searchbox", { name: "搜索模型", exact: true })
          .fill("copilot/");
        await p
          .getByRole("button", { name: "查看模型 gpt-fixture", exact: true })
          .waitFor();
        assert.equal(await p.locator(".mc-model-list-row").count(), 2);
        await p.screenshot({ path: path.join(out, "public-model-names.png") });
        await p
          .getByRole("button", { name: "查看模型 gpt-fixture", exact: true })
          .click();
        await p
          .getByRole("button", { name: "复制模型 ID gpt-fixture", exact: true })
          .click();
        assert.equal(
          await p.evaluate(() => window.__copiedFixture),
          "gpt-fixture",
        );
        await p.getByText("调用与路由设置", { exact: true }).click();
        assert(
          await p
            .getByRole("combobox", {
              name: "Endpoint copilot/gpt-fixture",
              exact: true,
            })
            .isDisabled(),
        );
        assert(
          await p
            .getByRole("button", {
              name: "路由策略 copilot/gpt-fixture",
              exact: true,
            })
            .isDisabled(),
        );
        await p.getByRole("button", { name: "Provider", exact: true }).click();
        await p
          .getByRole("button", { name: "选择来源 GitHub Copilot", exact: true })
          .click();
        await p
          .getByRole("button", { name: "删除来源 GitHub Copilot", exact: true })
          .click();
        await p
          .getByRole("dialog")
          .getByRole("button", { name: "确认删除", exact: true })
          .click();
        await p
          .getByRole("button", { name: "选择来源 GitHub Copilot", exact: true })
          .waitFor({ state: "hidden" });
        assert(
          (await p.evaluate(() => window.__fixtureCalls)).includes(
            "manual_copilot_disconnect",
          ),
        );
        report.flows.push(
          "login → discover → independent model catalog → managed endpoint → explicit disconnect",
        );
      } else if (mode === "denied") {
        await p
          .getByRole("alert")
          .filter({ hasText: "Copilot 权限不足" })
          .waitFor();
        await p.screenshot({
          path: path.join(out, "authorization-denied.png"),
        });
        assert(
          !(await p.evaluate(() => window.__fixtureCalls)).includes(
            "manual_discover",
          ),
        );
        report.flows.push(
          "permission failure does not discover or test models",
        );
      } else if (mode === "expired") {
        await p
          .getByRole("alert")
          .filter({ hasText: "授权码已过期" })
          .waitFor();
        assert(
          await p
            .getByRole("button", { name: "重新生成授权码", exact: true })
            .isVisible(),
        );
        assert(
          !(await p.evaluate(() => window.__fixtureCalls)).includes(
            "manual_copilot_poll",
          ),
        );
        report.flows.push("expired challenge is cancelled before polling");
      } else {
        await dialog.waitFor({ state: "hidden" });
        const card = p.getByRole("region", { name: "来源 GitHub Copilot" });
        await card.waitFor();
        assert((await card.innerText()).includes("fixture-user"));
        await card.getByRole("alert").waitFor();
        await p.screenshot({ path: path.join(out, "catalog-recovery.png") });
        report.flows.push(
          "catalog failure retains login and shows retry entry",
        );
      }
      const calls = await p.evaluate(() => window.__fixtureCalls);
      for (const forbidden of [
        "manual_test",
        "manual_apply_sync",
        "manual_launch",
        "manual_gateway",
      ])
        assert(!calls.includes(forbidden));
      await p.close();
    }
    report.checkedAt = new Date().toISOString();
    fs.writeFileSync(
      path.join(out, "checks.json"),
      JSON.stringify(report, null, 2),
    );
    assert.deepEqual(report.errors, []);
    assert.deepEqual(report.external, []);
    assert(
      report.accessibility.every((s) => !s.violations.length),
      "axe findings saved in checks.json",
    );
    console.log(JSON.stringify(report, null, 2));
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
