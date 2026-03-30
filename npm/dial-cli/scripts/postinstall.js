#!/usr/bin/env node

import fs from "node:fs/promises";
import { createWriteStream } from "node:fs";
import os from "node:os";
import path from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";

import extractZip from "extract-zip";
import * as tar from "tar";

import { buildDownloadUrl, installedBinaryPath, packageRootFrom, readPackageVersion, vendorDirectory } from "../lib/package-paths.mjs";
import { resolveAssetSpec } from "../lib/platform.mjs";

async function downloadFile(url, destinationPath) {
  const response = await fetch(url, {
    headers: {
      "user-agent": "dial-cli-npm-installer",
    },
  });

  if (!response.ok || !response.body) {
    const detail =
      response.status === 404
        ? "matching GitHub release asset was not found"
        : `download failed with HTTP ${response.status}`;
    throw new Error(`Unable to download ${url}: ${detail}`);
  }

  await pipeline(
    Readable.fromWeb(response.body),
    createWriteStream(destinationPath),
  );
}

async function extractArchive(archivePath, archiveType, destinationDir) {
  if (archiveType === "tar.gz") {
    await tar.x({
      cwd: destinationDir,
      file: archivePath,
      strict: true,
    });
    return;
  }

  if (archiveType === "zip") {
    await extractZip(archivePath, { dir: destinationDir });
    return;
  }

  throw new Error(`Unsupported archive type '${archiveType}'`);
}

async function main() {
  if (process.env.DIAL_NPM_SKIP_DOWNLOAD === "1") {
    console.log("dial-cli: skipping binary download because DIAL_NPM_SKIP_DOWNLOAD=1");
    return;
  }

  const packageRoot = packageRootFrom(import.meta.url);
  const version = await readPackageVersion(packageRoot);
  const asset = resolveAssetSpec();
  const downloadUrl = buildDownloadUrl(version, asset.assetName);
  const targetBinary = installedBinaryPath(packageRoot, asset.binaryName);
  const targetVendorDir = vendorDirectory(packageRoot);
  const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), "dial-cli-"));
  const archivePath = path.join(tempDir, asset.assetName);

  try {
    console.log(`dial-cli: downloading ${asset.assetName}`);
    await downloadFile(downloadUrl, archivePath);
    await extractArchive(archivePath, asset.archive, tempDir);
    await fs.mkdir(targetVendorDir, { recursive: true });

    const extractedBinary = path.join(tempDir, asset.binaryName);
    await fs.copyFile(extractedBinary, targetBinary);

    if (process.platform !== "win32") {
      await fs.chmod(targetBinary, 0o755);
    }

    console.log(`dial-cli: installed DIAL ${version} for ${asset.target}`);
  } catch (error) {
    console.error(`dial-cli: ${error.message}`);
    console.error(
      "dial-cli: install failed. Ensure the matching GitHub release exists and try the install again.",
    );
    process.exitCode = 1;
  } finally {
    await fs.rm(tempDir, { force: true, recursive: true });
  }
}

await main();
