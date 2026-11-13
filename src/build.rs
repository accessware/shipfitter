//! Helpers for build scripts.

use std::{
	env, fs,
	path::{Path, PathBuf},
	process::Command,
};

use crate::{Result, arch_suffix};

/// The profile directory the binary is linked into, such as `target/release`, or
/// `target/<triple>/release` for a cross build. Build scripts put generated files there so they
/// sit next to the executable.
#[must_use]
pub fn target_profile_dir() -> Option<PathBuf> {
	let var = |name| env::var_os(name).map(PathBuf::from);
	profile_dir_from(&env::var("PROFILE").ok()?, var("OUT_DIR"), var("CARGO_TARGET_DIR"))
}

/// Found from `OUT_DIR`, `<profile dir>/build/<crate>-<hash>/out`, which is absolute and keeps a
/// cross build's target triple. `CARGO_TARGET_DIR` is only the fallback: it leaves the triple out,
/// and a relative one is relative to where Cargo was started, not to the build script.
fn profile_dir_from(profile: &str, out_dir: Option<PathBuf>, target_dir: Option<PathBuf>) -> Option<PathBuf> {
	match out_dir {
		Some(out_dir) => {
			out_dir.ancestors().find(|dir| dir.file_name().is_some_and(|name| name == profile)).map(Path::to_path_buf)
		}
		None => Some(target_dir?.join(profile)),
	}
}

/// [`arch_suffix`] for the architecture being built for.
#[must_use]
pub fn target_arch_suffix() -> String {
	arch_suffix(&env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default()).to_string()
}

/// The commit being built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
	/// The full hash, or `unknown` outside a Git checkout.
	pub hash: String,
	/// The first seven characters of the hash.
	pub short_hash: String,
	/// Whether the commit has no tag, so it's a development build rather than a release.
	pub is_dev: bool,
}

fn git(args: &[&str]) -> Option<String> {
	let output = Command::new("git").args(args).output().ok()?;
	output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[must_use]
pub fn commit_info() -> CommitInfo {
	let hash = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
	let short_hash = hash.chars().take(7).collect();
	let is_dev = git(&["describe", "--tags", "--exact-match", "HEAD"]).is_none();
	CommitInfo { hash, short_hash, is_dev }
}

/// Sets `{prefix}_COMMIT_HASH`, `{prefix}_SHORT_HASH` and `{prefix}_IS_DEV` (`1` or `0`) for
/// `env!`, and reruns the build script when the checked out commit changes.
pub fn embed_commit_info(prefix: &str) -> CommitInfo {
	let info = commit_info();
	println!("cargo:rustc-env={prefix}_COMMIT_HASH={}", info.hash);
	println!("cargo:rustc-env={prefix}_SHORT_HASH={}", info.short_hash);
	println!("cargo:rustc-env={prefix}_IS_DEV={}", u8::from(info.is_dev));
	if let Some(git_dir) = git(&["rev-parse", "--absolute-git-dir"]).map(PathBuf::from) {
		println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
		println!("cargo:rerun-if-changed={}", git_dir.join("packed-refs").display());
		if let Some(head_ref) = git(&["symbolic-ref", "-q", "HEAD"]) {
			println!("cargo:rerun-if-changed={}", git_dir.join(head_ref).display());
		}
	}
	info
}

/// Copies `template` to `output`, replacing each `@NAME@` with its value.
///
/// This works like `configure_file` in `CMake`, and reruns the build script when the template
/// changes.
///
/// These are always filled in, before `replacements`:
///
/// - `@PROJECT_VERSION@`: the crate's version.
/// - `@ARCH_SUFFIX@`: the [`target_arch_suffix`].
/// - `@ARCH_ISS@`, `@ARCH_ALLOWED@` and `@ARCH_MODE@`: the Inno Setup architecture identifier,
///   `arm64` or `x64compatible`, for `ArchitecturesAllowed` and
///   `ArchitecturesInstallIn64BitMode`.
pub fn configure_file(template: &Path, output: &Path, replacements: &[(&str, &str)]) -> Result<()> {
	println!("cargo:rerun-if-changed={}", template.display());
	let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
	let arch_suffix = target_arch_suffix();
	let arch_iss = if arch_suffix == "arm64" { "arm64" } else { "x64compatible" };
	let standard = [
		("PROJECT_VERSION", version.as_str()),
		("ARCH_SUFFIX", arch_suffix.as_str()),
		("ARCH_ISS", arch_iss),
		("ARCH_ALLOWED", arch_iss),
		("ARCH_MODE", arch_iss),
	];
	let mut content = fs::read_to_string(template)?;
	for (name, value) in standard.iter().chain(replacements) {
		content = content.replace(&format!("@{name}@"), value);
	}
	fs::write(output, content)?;
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_profile_dir_is_found_from_out_dir() {
		let target = env::temp_dir().join("target");
		let out_dir = target.join("release").join("build").join("app-1234").join("out");
		assert_eq!(profile_dir_from("release", Some(out_dir), None), Some(target.join("release")));
	}

	/// Cargo 1.100 nests a build script's `OUT_DIR` one deeper, `build/<crate>/<hash>/out`.
	#[test]
	fn the_profile_dir_is_found_from_cargo_1_100s_out_dir() {
		let target = env::temp_dir().join("target");
		let profile = target.join("aarch64-apple-darwin").join("release");
		let out_dir = profile.join("build").join("app").join("1234").join("out");
		assert_eq!(profile_dir_from("release", Some(out_dir), None), Some(profile));
	}

	#[test]
	fn a_cross_build_keeps_its_target_triple_over_cargo_target_dir() {
		let target = env::temp_dir().join("target");
		let profile = target.join("aarch64-pc-windows-msvc").join("release");
		let out_dir = profile.join("build").join("app-1234").join("out");
		assert_eq!(profile_dir_from("release", Some(out_dir), Some(target)), Some(profile));
	}

	#[test]
	fn cargo_target_dir_is_the_fallback() {
		let target = env::temp_dir().join("target");
		assert_eq!(profile_dir_from("debug", None, Some(target.clone())), Some(target.join("debug")));
		assert_eq!(profile_dir_from("debug", None, None), None);
	}
}
