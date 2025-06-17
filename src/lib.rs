//! Library API for Terraform ephemeral deployments.
//!
//! Exposes two functions:
//! - `deploy`: applies a Terraform configuration and returns its outputs
//! - `undeploy`: destroys an existing Terraform configuration

use anyhow::{Context, Result, bail};
use serde_json::{self, Value};
use std::{
  collections::HashMap,
  env,
  fs,
  path::{Path, PathBuf},
  process::{Command, Stdio},
};
use sha2::{Digest, Sha256};

fn ensure_terraform_installed() -> Result<()> {
  let status = Command::new("terraform")
    .arg("-version")
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .context("Failed to execute `terraform -version`")?;
  if !status.success() {
    bail!("Terraform must be installed and in PATH");
  }
  Ok(())
}

/// Recursively copy a directory tree from `src` to `dst`.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
  fs::create_dir_all(dst).with_context(|| format!("Failed to create directory {:?}", dst))?;
  for entry in fs::read_dir(src).with_context(|| format!("Failed to read directory {:?}", src))? {
    let entry = entry.with_context(|| format!("Failed to access entry in {:?}", src))?;
    let path = entry.path();
    let dest = dst.join(entry.file_name());
    if path.is_dir() {
      copy_dir_recursive(&path, &dest)?;
    } else {
      fs::copy(&path, &dest)
        .with_context(|| format!("Failed to copy file {:?} to {:?}", path, dest))?;
    }
  }
  Ok(())
}

/// Prepare a deterministic temp workspace based on the source directory path.
fn prepare_work_dir(src_dir: &Path) -> Result<PathBuf> {
  let mut hasher = Sha256::new();
  hasher.update(src_dir.to_string_lossy().as_bytes());
  let hash = format!("{:x}", hasher.finalize());
  let work = env::temp_dir().join("atar").join(hash);
  if !work.exists() {
    println!("Copying Terraform files to temporary directory {}", work.display());
    copy_dir_recursive(src_dir, &work)?;
  }
  Ok(work)
}

/// Apply Terraform config at `file` with provided `vars`.
///
/// Returns a map from output names to their stringified values.
pub fn deploy<P: AsRef<Path>>(
  file: P,
  vars: &HashMap<String, String>,
  debug: bool,
) -> Result<HashMap<String, String>> {
  ensure_terraform_installed()?;
  let file = file
    .as_ref()
    .canonicalize()
    .context("Failed to canonicalize Terraform path")?;
  let src_dir = file
    .parent()
    .context("Cannot determine Terraform directory")?;
  let work_dir = prepare_work_dir(src_dir)?;

  // init
  println!("Initializing Terraform...");

  let mut init = Command::new("terraform");
  init.current_dir(&work_dir).arg("init");
  if !debug {
    init.stdout(Stdio::null()).stderr(Stdio::null());
  }
  let status = init
    .status()
    .context("Failed to execute `terraform init`")?;
  if !status.success() {
    bail!("`terraform init` failed with exit code {}", status);
  }

  println!("Applying Terraform...");
  {
    let mut cmd = Command::new("terraform");
    cmd.current_dir(&work_dir).arg("apply").arg("-auto-approve");
    for (k, v) in vars {
      cmd.arg("-var").arg(format!("{}={}", k, v));
    }
    if !debug {
      cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let status = cmd
      .status()
      .context("Failed to execute `terraform apply`")?;
    if !status.success() {
      bail!("`terraform apply` failed with exit code {}", status);
    }
  }

  // output JSON
  let output = Command::new("terraform")
    .current_dir(&work_dir)
    .arg("output")
    .arg("-json")
    .output()
    .context("Failed to execute `terraform output -json`")?;
  if !output.status.success() {
    bail!(
      "`terraform output -json` failed with exit code {}",
      output.status
    );
  }
  let raw: HashMap<String, Value> = serde_json::from_slice(&output.stdout)
    .context("Failed to parse Terraform output JSON")?;
  let mut results = HashMap::new();
  for (key, val) in raw {
    if let Some(inner) = val.get("value") {
      let s = if inner.is_string() {
        inner.as_str().unwrap().to_string()
      } else {
        inner.to_string()
      };
      results.insert(key, s);
    }
  }
  Ok(results)
}

/// Destroy Terraform config at `file` with provided `vars`.
pub fn undeploy<P: AsRef<Path>>(
  file: P,
  vars: &HashMap<String, String>,
  debug: bool,
) -> Result<()> {
  ensure_terraform_installed()?;
  let file = file
    .as_ref()
    .canonicalize()
    .context("Failed to canonicalize Terraform path")?;
  let src_dir = file
    .parent()
    .context("Cannot determine Terraform directory")?;
  let work_dir = prepare_work_dir(src_dir)?;

  println!("Destroying Terraform...");

  let mut cmd = Command::new("terraform");
  cmd.current_dir(&work_dir).arg("destroy").arg("-auto-approve");
  for (k, v) in vars {
    cmd.arg("-var").arg(format!("{}={}", k, v));
  }
  if !debug {
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
  }
  let status = cmd
    .status()
    .context("Failed to execute `terraform destroy`")?;
  if !status.success() {
    bail!("`terraform destroy` failed with exit code {}", status);
  }
  println!("All resources have been destroyed.");
  Ok(())
}
