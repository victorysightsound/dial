# @victorysightsound/dial-cli

Install the DIAL CLI from npm:

```bash
npm install -g @victorysightsound/dial-cli
```

This package downloads the matching prebuilt DIAL binary from the GitHub release for the current package version and exposes `dial` as the global command.

Supported targets:
- macOS Apple Silicon
- macOS Intel
- Linux x86_64
- Linux ARM64
- Windows x86_64

Requirements:
- Node.js 18 or newer
- internet access during install so the matching GitHub release asset can be downloaded

Verify the install:

```bash
dial --version
```

If you need the full onboarding guide, see the main project docs:
- https://github.com/victorysightsound/dial/blob/main/docs/getting-started.md
