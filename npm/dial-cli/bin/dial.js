#!/usr/bin/env node

import { spawn } from "node:child_process";
import fs from "node:fs";

import { installedBinaryPath, packageRootFrom } from "../lib/package-paths.mjs";
import { resolveAssetSpec } from "../lib/platform.mjs";

let asset;
try {
  asset = resolveAssetSpec();
} catch (error) {
  console.error(`dial-cli: ${error.message}`);
  process.exit(1);
}

const packageRoot = packageRootFrom(import.meta.url);
const binaryPath = installedBinaryPath(packageRoot, asset.binaryName);

if (!fs.existsSync(binaryPath)) {
  console.error(
    `dial-cli: missing platform binary at ${binaryPath}. Reinstall with 'npm install -g getdial'.`,
  );
  process.exit(1);
}

const child = spawn(binaryPath, process.argv.slice(2), {
  stdio: "inherit",
});

child.on("error", (error) => {
  console.error(`dial-cli: failed to start '${binaryPath}': ${error.message}`);
  process.exit(1);
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }

  process.exit(code ?? 1);
});
