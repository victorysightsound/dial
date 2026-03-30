const TARGETS = new Map([
  [
    "darwin:arm64",
    {
      target: "aarch64-apple-darwin",
      archive: "tar.gz",
      binaryName: "dial",
    },
  ],
  [
    "darwin:x64",
    {
      target: "x86_64-apple-darwin",
      archive: "tar.gz",
      binaryName: "dial",
    },
  ],
  [
    "linux:arm64",
    {
      target: "aarch64-unknown-linux-gnu",
      archive: "tar.gz",
      binaryName: "dial",
    },
  ],
  [
    "linux:x64",
    {
      target: "x86_64-unknown-linux-gnu",
      archive: "tar.gz",
      binaryName: "dial",
    },
  ],
  [
    "win32:x64",
    {
      target: "x86_64-pc-windows-msvc",
      archive: "zip",
      binaryName: "dial.exe",
    },
  ],
]);

export function supportedTargetKeys() {
  return [...TARGETS.keys()].sort();
}

export function resolveAssetSpec(
  platform = process.platform,
  arch = process.arch,
) {
  const key = `${platform}:${arch}`;
  const spec = TARGETS.get(key);
  if (!spec) {
    throw new Error(
      `Unsupported platform '${platform}' on '${arch}'. Supported targets: ${supportedTargetKeys().join(", ")}`,
    );
  }

  return {
    ...spec,
    assetName: `dial-${spec.target}.${spec.archive}`,
  };
}
