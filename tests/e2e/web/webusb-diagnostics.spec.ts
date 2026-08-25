import { expect, test } from "@playwright/test";

test("webusb diagnostics page loads (no hardware required)", async ({ page }) => {
  await page.goto("/apps/web/webusb_diagnostics.html", { waitUntil: "load" });

  await expect(page.getByRole("heading", { name: "WebUSB diagnostics / enumeration" })).toBeVisible();

  const secure = await page.evaluate(() => globalThis.isSecureContext);
  const hasUsb = await page.evaluate(() => !!(navigator as any).usb);

  if (!secure) {
    await expect(page.getByText("Secure context required")).toBeVisible();
    return;
  }

  if (!hasUsb) {
    await expect(page.getByText("WebUSB unavailable")).toBeVisible();
    return;
  }

  // WebUSB exists: ensure the basic UI is present (but do not call requestDevice).
  await expect(page.getByRole("button", { name: "Request USB device" })).toBeVisible();
  // The `+` is escaped once, not twice: `\\+` would match a run of literal backslashes,
  // which the button text does not contain, so this could never match.
  await expect(page.getByRole("button", { name: /Try open \+ claim/ })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Previously granted devices" })).toBeVisible();
});
