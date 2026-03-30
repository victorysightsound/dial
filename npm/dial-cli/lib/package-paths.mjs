import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

export function packageRootFrom(importMetaUrl) {
  return path.resolve(path.dirname(fileURLToPath(importMetaUrl)), "..");
}

export function vendorDirectory(packageRoot) {
  return path.join(packageRoot, "vendor");
}

export function installedBinaryPath(packageRoot, binaryName) {
  return path.join(vendorDirectory(packageRoot), binaryName);
}

export async function readPackageVersion(packageRoot) {
  const packageJsonPath = path.join(packageRoot, "package.json");
  const packageJson = JSON.parse(await fs.readFile(packageJsonPath, "utf8"));
  return String(packageJson.version);
}

export function buildDownloadUrl(version, assetName, env = process.env) {
  if (env.DIAL_NPM_BINARY_URL) {
    return env.DIAL_NPM_BINARY_URL;
  }

  const normalizedVersion = String(version).replace(/^v/, "");
  const baseUrl =
    env.DIAL_NPM_BASE_URL ??
    `https://github.com/victorysightsound/dial/releases/download/v${normalizedVersion}`;

  return `${baseUrl.replace(/\/$/, "")}/${assetName}`;
}
