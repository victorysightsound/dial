import test from "node:test";
import assert from "node:assert/strict";

import { buildDownloadUrl, installedBinaryPath } from "../lib/package-paths.mjs";
import { resolveAssetSpec, supportedTargetKeys } from "../lib/platform.mjs";

test("resolveAssetSpec maps macOS arm64", () => {
  const spec = resolveAssetSpec("darwin", "arm64");
  assert.equal(spec.target, "aarch64-apple-darwin");
  assert.equal(spec.archive, "tar.gz");
  assert.equal(spec.binaryName, "dial");
  assert.equal(spec.assetName, "dial-aarch64-apple-darwin.tar.gz");
});

test("resolveAssetSpec maps windows x64", () => {
  const spec = resolveAssetSpec("win32", "x64");
  assert.equal(spec.target, "x86_64-pc-windows-msvc");
  assert.equal(spec.archive, "zip");
  assert.equal(spec.binaryName, "dial.exe");
  assert.equal(spec.assetName, "dial-x86_64-pc-windows-msvc.zip");
});

test("resolveAssetSpec rejects unsupported targets", () => {
  assert.throws(
    () => resolveAssetSpec("win32", "arm64"),
    /Unsupported platform/,
  );
  assert.ok(supportedTargetKeys().includes("linux:x64"));
});

test("buildDownloadUrl uses release defaults", () => {
  const url = buildDownloadUrl("4.2.5", "dial-x86_64-apple-darwin.tar.gz", {});
  assert.equal(
    url,
    "https://github.com/victorysightsound/dial/releases/download/v4.2.5/dial-x86_64-apple-darwin.tar.gz",
  );
});

test("buildDownloadUrl respects overrides", () => {
  const url = buildDownloadUrl("4.2.5", "dial.zip", {
    DIAL_NPM_BASE_URL: "https://example.com/releases/v4.2.5/",
  });
  assert.equal(url, "https://example.com/releases/v4.2.5/dial.zip");

  const directUrl = buildDownloadUrl("4.2.5", "ignored.zip", {
    DIAL_NPM_BINARY_URL: "https://example.com/custom/dial.zip",
  });
  assert.equal(directUrl, "https://example.com/custom/dial.zip");
});

test("installedBinaryPath uses vendor directory", () => {
  const binaryPath = installedBinaryPath("/tmp/dial-cli", "dial");
  assert.equal(binaryPath, "/tmp/dial-cli/vendor/dial");
});
