use anyhow::{anyhow, bail, Context, Result};
use dial_core::output;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const REPO_API_LATEST: &str = "https://api.github.com/repos/victorysightsound/dial/releases/latest";
const RELEASE_DOWNLOAD_BASE: &str = "https://github.com/victorysightsound/dial/releases/download";
const CARGO_PACKAGE: &str = "dial-cli";
const NPM_PACKAGE: &str = "getdial";
const LEGACY_NPM_PACKAGE: &str = "@victorysightsound/dial-cli";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallMethod {
    Cargo,
    Npm(&'static str),
    Binary,
}

impl InstallMethod {
    fn display_name(self) -> &'static str {
        match self {
            InstallMethod::Cargo => "cargo",
            InstallMethod::Npm(_) => "npm",
            InstallMethod::Binary => "binary",
        }
    }
}

#[derive(Debug, Clone)]
struct InstallInfo {
    method: InstallMethod,
    canonical_exe: PathBuf,
    alias_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseAsset {
    asset_name: String,
    binary_name: &'static str,
    archive_kind: ArchiveKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    TarGz,
    Zip,
}

pub async fn upgrade(requested_version: Option<&str>) -> Result<()> {
    let install = InstallInfo::detect()?;
    let target_version = match requested_version {
        Some(version) => normalize_version(version),
        None => fetch_latest_version().await?,
    };

    if target_version == dial_core::VERSION {
        output::print_info(&format!(
            "DIAL is already at {}. No upgrade needed.",
            target_version
        ));
        return Ok(());
    }

    println!(
        "{}",
        output::bold(&format!(
            "Upgrading DIAL from {} to {} ({})",
            dial_core::VERSION,
            target_version,
            install.method.display_name()
        ))
    );

    match install.method {
        InstallMethod::Cargo => upgrade_via_cargo(&target_version).await,
        InstallMethod::Npm(package_name) => upgrade_via_npm(package_name, &target_version).await,
        InstallMethod::Binary => upgrade_binary_install(&install, &target_version).await,
    }
}

pub async fn uninstall(yes: bool) -> Result<()> {
    let install = InstallInfo::detect()?;

    if !yes
        && !output::prompt_yes_no(
            "Uninstall DIAL from this machine? Project .dial directories will be left untouched.",
        )
    {
        output::print_info("Uninstall cancelled.");
        return Ok(());
    }

    println!(
        "{}",
        output::bold(&format!(
            "Removing DIAL installed via {}",
            install.method.display_name()
        ))
    );

    match install.method {
        InstallMethod::Cargo => uninstall_via_cargo(&install).await,
        InstallMethod::Npm(package_name) => uninstall_via_npm(package_name, &install).await,
        InstallMethod::Binary => uninstall_binary_install(&install).await,
    }
}

impl InstallInfo {
    fn detect() -> Result<Self> {
        let current_exe =
            env::current_exe().context("Failed to locate the current DIAL executable")?;
        let canonical_exe = fs::canonicalize(&current_exe).unwrap_or_else(|_| current_exe.clone());
        let method = detect_install_method(&canonical_exe);
        let alias_paths = discover_alias_paths(&canonical_exe);

        Ok(Self {
            method,
            canonical_exe,
            alias_paths,
        })
    }
}

fn detect_install_method(path: &Path) -> InstallMethod {
    let parts = path_components_lower(path);
    if path_ends_with(&parts, &[".cargo", "bin"], "dial") {
        InstallMethod::Cargo
    } else if path_ends_with(&parts, &["node_modules", "getdial", "vendor"], "dial") {
        InstallMethod::Npm(NPM_PACKAGE)
    } else if path_ends_with(
        &parts,
        &["node_modules", "@victorysightsound", "dial-cli", "vendor"],
        "dial",
    ) {
        InstallMethod::Npm(LEGACY_NPM_PACKAGE)
    } else {
        InstallMethod::Binary
    }
}

fn path_components_lower(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect()
}

fn path_ends_with(parts: &[String], prefix: &[&str], binary_stem: &str) -> bool {
    if parts.len() < prefix.len() + 1 {
        return false;
    }

    let suffix = &parts[parts.len() - (prefix.len() + 1)..];
    let file_name = suffix.last().expect("suffix always has final component");
    let stem_matches = file_name == binary_stem || file_name == &format!("{binary_stem}.exe");
    stem_matches
        && suffix[..prefix.len()]
            .iter()
            .zip(prefix.iter())
            .all(|(actual, expected)| actual == expected)
}

fn discover_alias_paths(canonical_exe: &Path) -> Vec<PathBuf> {
    let mut matches = Vec::new();
    let Some(path_var) = env::var_os("PATH") else {
        return matches;
    };

    let candidate_names: &[&str] = if cfg!(windows) {
        &["dial.exe", "dial.cmd", "dial.ps1", "dial.bat"]
    } else {
        &["dial"]
    };

    for dir in env::split_paths(&path_var) {
        for name in candidate_names {
            let candidate = dir.join(name);
            if !candidate.exists() {
                continue;
            }

            let resolved = fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
            if resolved == canonical_exe
                && candidate != canonical_exe
                && !matches.contains(&candidate)
            {
                matches.push(candidate);
            }
        }
    }

    matches
}

async fn fetch_latest_version() -> Result<String> {
    let client = reqwest::Client::builder()
        .build()
        .context("Failed to build HTTP client")?;
    let response = client
        .get(REPO_API_LATEST)
        .header(USER_AGENT, "dial-cli/self-manage")
        .header(ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .context("Failed to query the latest DIAL release")?
        .error_for_status()
        .context("GitHub did not return a successful latest-release response")?;

    let payload: Value = response
        .json()
        .await
        .context("Failed to parse the latest-release response")?;
    let tag_name = payload
        .get("tag_name")
        .and_then(|value| value.as_str())
        .ok_or_else(|| anyhow!("GitHub release response did not include tag_name"))?;

    Ok(normalize_version(tag_name))
}

fn normalize_version(version: &str) -> String {
    version.trim().trim_start_matches('v').to_string()
}

async fn upgrade_via_cargo(target_version: &str) -> Result<()> {
    if cfg!(windows) {
        ensure_command_available("cargo", "--version")?;
        let script = build_windows_command_script(
            "upgrade",
            &format!("cargo install {CARGO_PACKAGE} --force --version {target_version}"),
            &format!("DIAL upgraded to {target_version}."),
        );
        spawn_windows_followup(&script)?;
        output::print_success(
            "DIAL upgrade has been scheduled in a new window. Let this command exit before the installer finishes.",
        );
        return Ok(());
    }

    run_command_streaming(
        "cargo",
        &[
            "install",
            CARGO_PACKAGE,
            "--force",
            "--version",
            target_version,
        ],
    )?;
    output::print_success(&format!("DIAL upgraded to {}.", target_version));
    Ok(())
}

async fn upgrade_via_npm(package_name: &str, target_version: &str) -> Result<()> {
    if cfg!(windows) {
        ensure_command_available("npm", "--version")?;
        let script = build_windows_command_script(
            "upgrade",
            &format!("npm install -g {package_name}@{target_version}"),
            &format!("DIAL upgraded to {target_version}."),
        );
        spawn_windows_followup(&script)?;
        output::print_success(
            "DIAL upgrade has been scheduled in a new window. Let this command exit before the installer finishes.",
        );
        return Ok(());
    }

    let package_spec = format!("{package_name}@{target_version}");
    run_command_streaming("npm", &["install", "-g", &package_spec])?;
    output::print_success(&format!("DIAL upgraded to {}.", target_version));
    Ok(())
}

async fn upgrade_binary_install(install: &InstallInfo, target_version: &str) -> Result<()> {
    let asset = release_asset_for_current_platform()?;
    let temp_dir = make_temp_dir("dial-upgrade")?;
    let archive_path = temp_dir.join(&asset.asset_name);
    let extracted_binary = temp_dir.join(asset.binary_name);
    let download_url = build_release_download_url(target_version, &asset.asset_name);

    download_file(&download_url, &archive_path).await?;
    extract_release_archive(&archive_path, asset.archive_kind, &temp_dir)?;

    if !extracted_binary.exists() {
        bail!(
            "Downloaded release did not contain the expected binary '{}'",
            asset.binary_name
        );
    }

    if cfg!(windows) {
        let script = build_windows_binary_replace_script(
            &install.canonical_exe,
            &extracted_binary,
            target_version,
        );
        spawn_windows_followup(&script)?;
        output::print_success(
            "DIAL upgrade has been scheduled in a new window. Let this command exit before the installer finishes.",
        );
        return Ok(());
    }

    replace_binary_unix(&install.canonical_exe, &extracted_binary)?;
    output::print_success(&format!("DIAL upgraded to {}.", target_version));
    Ok(())
}

async fn uninstall_via_cargo(install: &InstallInfo) -> Result<()> {
    if cfg!(windows) {
        ensure_command_available("cargo", "--version")?;
        let script = build_windows_command_script(
            "uninstall",
            &format!("cargo uninstall {CARGO_PACKAGE}"),
            "DIAL removed. Existing project .dial directories were left untouched.",
        );
        spawn_windows_followup(&script)?;
        output::print_success(
            "DIAL uninstall has been scheduled in a new window. Existing project .dial directories were left untouched.",
        );
        return Ok(());
    }

    run_command_streaming("cargo", &["uninstall", CARGO_PACKAGE])?;
    cleanup_alias_paths(&install.alias_paths)?;
    output::print_success("DIAL removed. Existing project .dial directories were left untouched.");
    Ok(())
}

async fn uninstall_via_npm(package_name: &str, _install: &InstallInfo) -> Result<()> {
    if cfg!(windows) {
        ensure_command_available("npm", "--version")?;
        let script = build_windows_command_script(
            "uninstall",
            &format!("npm uninstall -g {package_name}"),
            "DIAL removed. Existing project .dial directories were left untouched.",
        );
        spawn_windows_followup(&script)?;
        output::print_success(
            "DIAL uninstall has been scheduled in a new window. Existing project .dial directories were left untouched.",
        );
        return Ok(());
    }

    run_command_streaming("npm", &["uninstall", "-g", package_name])?;
    output::print_success("DIAL removed. Existing project .dial directories were left untouched.");
    Ok(())
}

async fn uninstall_binary_install(install: &InstallInfo) -> Result<()> {
    if cfg!(windows) {
        let script =
            build_windows_binary_uninstall_script(&install.canonical_exe, &install.alias_paths);
        spawn_windows_followup(&script)?;
        output::print_success(
            "DIAL uninstall has been scheduled in a new window. Existing project .dial directories were left untouched.",
        );
        return Ok(());
    }

    cleanup_alias_paths(&install.alias_paths)?;
    fs::remove_file(&install.canonical_exe).with_context(|| {
        format!(
            "Failed to remove '{}'. Try re-running with the permissions used to install it.",
            install.canonical_exe.display()
        )
    })?;
    output::print_success("DIAL removed. Existing project .dial directories were left untouched.");
    Ok(())
}

fn cleanup_alias_paths(paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        if path.exists() {
            fs::remove_file(path)
                .with_context(|| format!("Failed to remove linked command '{}'", path.display()))?;
        }
    }
    Ok(())
}

fn ensure_command_available(command: &str, version_flag: &str) -> Result<()> {
    let status = Command::new(command)
        .arg(version_flag)
        .status()
        .with_context(|| format!("'{}' is not installed or not on PATH", command))?;

    if status.success() {
        Ok(())
    } else {
        bail!(
            "'{} {}' failed. Fix that tool first, then retry.",
            command,
            version_flag
        );
    }
}

fn run_command_streaming(command: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(command)
        .args(args)
        .status()
        .with_context(|| format!("Failed to start '{}'", command))?;

    if status.success() {
        Ok(())
    } else {
        bail!(
            "'{} {}' exited with status {}",
            command,
            args.join(" "),
            status
        );
    }
}

fn build_release_download_url(version: &str, asset_name: &str) -> String {
    format!(
        "{}/v{}/{}",
        RELEASE_DOWNLOAD_BASE,
        normalize_version(version),
        asset_name
    )
}

fn release_asset_for_current_platform() -> Result<ReleaseAsset> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;

    match (os, arch) {
        ("macos", "aarch64") => Ok(ReleaseAsset {
            asset_name: "dial-aarch64-apple-darwin.tar.gz".to_string(),
            binary_name: "dial",
            archive_kind: ArchiveKind::TarGz,
        }),
        ("macos", "x86_64") => Ok(ReleaseAsset {
            asset_name: "dial-x86_64-apple-darwin.tar.gz".to_string(),
            binary_name: "dial",
            archive_kind: ArchiveKind::TarGz,
        }),
        ("linux", "x86_64") => Ok(ReleaseAsset {
            asset_name: "dial-x86_64-unknown-linux-gnu.tar.gz".to_string(),
            binary_name: "dial",
            archive_kind: ArchiveKind::TarGz,
        }),
        ("linux", "aarch64") => Ok(ReleaseAsset {
            asset_name: "dial-aarch64-unknown-linux-gnu.tar.gz".to_string(),
            binary_name: "dial",
            archive_kind: ArchiveKind::TarGz,
        }),
        ("windows", "x86_64") => Ok(ReleaseAsset {
            asset_name: "dial-x86_64-pc-windows-msvc.zip".to_string(),
            binary_name: "dial.exe",
            archive_kind: ArchiveKind::Zip,
        }),
        _ => bail!(
            "Unsupported platform for automatic upgrade: {} {}",
            os,
            arch
        ),
    }
}

async fn download_file(url: &str, destination: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .build()
        .context("Failed to build HTTP client")?;
    let response = client
        .get(url)
        .header(USER_AGENT, "dial-cli/self-manage")
        .send()
        .await
        .with_context(|| format!("Failed to download {}", url))?
        .error_for_status()
        .with_context(|| format!("Failed to download {}", url))?;
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("Failed to read {}", url))?;
    fs::write(destination, bytes.as_ref())
        .with_context(|| format!("Failed to write '{}'", destination.display()))?;
    Ok(())
}

fn extract_release_archive(
    archive_path: &Path,
    kind: ArchiveKind,
    destination_dir: &Path,
) -> Result<()> {
    match kind {
        ArchiveKind::TarGz => {
            let status = Command::new("tar")
                .args(["-xzf"])
                .arg(archive_path)
                .args(["-C"])
                .arg(destination_dir)
                .status()
                .context("Failed to launch 'tar' to extract the release archive")?;
            if !status.success() {
                bail!("Archive extraction failed for '{}'", archive_path.display());
            }
        }
        ArchiveKind::Zip => {
            let status = Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    "Expand-Archive -LiteralPath $args[0] -DestinationPath $args[1] -Force",
                ])
                .arg(archive_path)
                .arg(destination_dir)
                .status()
                .context("Failed to launch PowerShell to extract the release archive")?;
            if !status.success() {
                bail!("Archive extraction failed for '{}'", archive_path.display());
            }
        }
    }

    Ok(())
}

fn replace_binary_unix(target_path: &Path, downloaded_binary: &Path) -> Result<()> {
    let parent = target_path.parent().ok_or_else(|| {
        anyhow!(
            "Cannot determine installation directory for '{}'",
            target_path.display()
        )
    })?;
    let temp_target = parent.join(format!(
        ".{}.upgrade",
        target_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
    ));

    fs::copy(downloaded_binary, &temp_target).with_context(|| {
        format!(
            "Failed to copy the upgraded binary into '{}'",
            temp_target.display()
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(target_path)
            .map(|meta| meta.permissions().mode())
            .unwrap_or(0o755);
        fs::set_permissions(&temp_target, fs::Permissions::from_mode(mode)).with_context(|| {
            format!(
                "Failed to set execute permissions on '{}'",
                temp_target.display()
            )
        })?;
    }

    fs::rename(&temp_target, target_path).with_context(|| {
        format!(
            "Failed to replace '{}'. Try re-running with the permissions used to install it.",
            target_path.display()
        )
    })?;

    Ok(())
}

fn make_temp_dir(prefix: &str) -> Result<PathBuf> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), unique));
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create temporary directory '{}'", dir.display()))?;
    Ok(dir)
}

fn build_windows_command_script(action: &str, command: &str, success_line: &str) -> String {
    format!(
        "@echo off\r\n\
setlocal\r\n\
title DIAL {action}\r\n\
echo Finishing DIAL {action}...\r\n\
timeout /t 2 /nobreak >nul\r\n\
{command}\r\n\
if errorlevel 1 (\r\n\
  echo.\r\n\
  echo DIAL {action} failed.\r\n\
  pause\r\n\
  exit /b 1\r\n\
)\r\n\
echo.\r\n\
echo {success_line}\r\n"
    )
}

fn build_windows_binary_replace_script(
    target: &Path,
    source: &Path,
    target_version: &str,
) -> String {
    format!(
        "@echo off\r\n\
setlocal\r\n\
set \"TARGET={}\"\r\n\
set \"SOURCE={}\"\r\n\
title DIAL upgrade\r\n\
echo Finishing DIAL upgrade to {}...\r\n\
set /a ATTEMPTS=0\r\n\
:retry\r\n\
move /Y \"%SOURCE%\" \"%TARGET%\" >nul 2>&1\r\n\
if exist \"%SOURCE%\" (\r\n\
  set /a ATTEMPTS+=1\r\n\
  if %ATTEMPTS% GEQ 60 (\r\n\
    echo.\r\n\
    echo Failed to replace %TARGET%.\r\n\
    echo The downloaded binary is still at %SOURCE%.\r\n\
    pause\r\n\
    exit /b 1\r\n\
  )\r\n\
  timeout /t 1 /nobreak >nul\r\n\
  goto retry\r\n\
)\r\n\
echo.\r\n\
echo DIAL upgraded to {}.\r\n",
        target.display(),
        source.display(),
        target_version,
        target_version
    )
}

fn build_windows_binary_uninstall_script(target: &Path, aliases: &[PathBuf]) -> String {
    let mut script = format!(
        "@echo off\r\n\
setlocal\r\n\
set \"TARGET={}\"\r\n\
title DIAL uninstall\r\n\
echo Finishing DIAL uninstall...\r\n\
set /a ATTEMPTS=0\r\n\
:retry\r\n\
del /F /Q \"%TARGET%\" >nul 2>&1\r\n\
if exist \"%TARGET%\" (\r\n\
  set /a ATTEMPTS+=1\r\n\
  if %ATTEMPTS% GEQ 60 (\r\n\
    echo.\r\n\
    echo Failed to remove %TARGET%.\r\n\
    pause\r\n\
    exit /b 1\r\n\
  )\r\n\
  timeout /t 1 /nobreak >nul\r\n\
  goto retry\r\n\
)\r\n",
        target.display()
    );

    for alias in aliases {
        script.push_str(&format!(
            "if exist \"{}\" del /F /Q \"{}\" >nul 2>&1\r\n",
            alias.display(),
            alias.display()
        ));
    }

    script.push_str(
        "echo.\r\n\
echo DIAL removed. Existing project .dial directories were left untouched.\r\n",
    );

    script
}

fn spawn_windows_followup(script_contents: &str) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

        let temp_dir = make_temp_dir("dial-self-manage")?;
        let script_path = temp_dir.join("dial-self-manage.cmd");
        let final_script = format!("{}del /F /Q \"%~f0\" >nul 2>&1\r\n", script_contents);
        fs::write(&script_path, final_script).with_context(|| {
            format!(
                "Failed to write Windows follow-up script '{}'",
                script_path.display()
            )
        })?;

        Command::new("cmd")
            .args(["/C"])
            .arg(script_path)
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .context("Failed to launch the Windows follow-up installer")?;
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = script_contents;
        bail!("Windows follow-up scripts are not available on this platform")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_install_method_recognizes_cargo_path() {
        let path = Path::new("/Users/test/.cargo/bin/dial");
        assert_eq!(detect_install_method(path), InstallMethod::Cargo);
    }

    #[test]
    fn detect_install_method_recognizes_npm_vendor_path() {
        let path = Path::new("/Users/test/.local/share/node_modules/getdial/vendor/dial");
        assert_eq!(detect_install_method(path), InstallMethod::Npm(NPM_PACKAGE));
    }

    #[test]
    fn detect_install_method_recognizes_legacy_npm_vendor_path() {
        let path = Path::new(
            "/Users/test/.local/share/node_modules/@victorysightsound/dial-cli/vendor/dial",
        );
        assert_eq!(
            detect_install_method(path),
            InstallMethod::Npm(LEGACY_NPM_PACKAGE)
        );
    }

    #[test]
    fn detect_install_method_defaults_to_binary() {
        let path = Path::new("/usr/local/bin/dial");
        assert_eq!(detect_install_method(path), InstallMethod::Binary);
    }

    #[test]
    fn normalize_version_trims_leading_v() {
        assert_eq!(normalize_version("v4.2.6"), "4.2.6");
        assert_eq!(normalize_version("4.2.6"), "4.2.6");
    }

    #[test]
    fn build_release_download_url_uses_versioned_tag() {
        assert_eq!(
            build_release_download_url("4.2.6", "dial-x86_64-apple-darwin.tar.gz"),
            "https://github.com/victorysightsound/dial/releases/download/v4.2.6/dial-x86_64-apple-darwin.tar.gz"
        );
    }

    #[test]
    fn build_windows_command_script_contains_command_and_success_text() {
        let script = build_windows_command_script(
            "upgrade",
            "cargo install dial-cli --force --version 4.2.6",
            "DIAL upgraded to 4.2.6.",
        );
        assert!(script.contains("cargo install dial-cli --force --version 4.2.6"));
        assert!(script.contains("DIAL upgraded to 4.2.6."));
    }

    #[test]
    fn build_windows_binary_replace_script_contains_paths() {
        let script = build_windows_binary_replace_script(
            Path::new("C:\\Users\\User\\dial.exe"),
            Path::new("C:\\Temp\\dial.exe"),
            "4.2.6",
        );
        assert!(script.contains("C:\\Users\\User\\dial.exe"));
        assert!(script.contains("C:\\Temp\\dial.exe"));
        assert!(script.contains("DIAL upgraded to 4.2.6."));
    }

    #[test]
    fn build_windows_binary_uninstall_script_cleans_aliases() {
        let script = build_windows_binary_uninstall_script(
            Path::new("C:\\Users\\User\\dial.exe"),
            &[PathBuf::from("C:\\Users\\User\\bin\\dial.exe")],
        );
        assert!(script.contains("C:\\Users\\User\\dial.exe"));
        assert!(script.contains("C:\\Users\\User\\bin\\dial.exe"));
        assert!(script.contains("Existing project .dial directories were left untouched."));
    }
}
