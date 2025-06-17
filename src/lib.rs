//! Library API for Terraform ephemeral deployments.
//!
//! Exposes two functions:
//! - `deploy`: applies a Terraform configuration and returns its outputs
//! - `undeploy`: destroys an existing Terraform configuration

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
  collections::HashMap,
  path::Path,
  process::{Command, Stdio},
};

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

/// Apply Terraform config at `file` with provided `vars`.
///
/// Returns a map from output names to their stringified values.
pub fn deploy<P: AsRef<Path>>(
  file: P,
  vars: &HashMap<String, String>,
) -> Result<HashMap<String, String>> {
  ensure_terraform_installed()?;
  let file = file
    .as_ref()
    .canonicalize()
    .context("Failed to canonicalize Terraform path")?;
  let dir = file
    .parent()
    .context("Cannot determine Terraform directory")?;

  // init
  let status = Command::new("terraform")
    .current_dir(dir)
    .arg("init")
    .status()
    .context("Failed to execute `terraform init`")?;
  if !status.success() {
    bail!("`terraform init` failed with exit code {}", status);
  }

  // apply
  {
    let mut cmd = Command::new("terraform");
    cmd.current_dir(dir).arg("apply").arg("-auto-approve");
    for (k, v) in vars {
      cmd.arg("-var").arg(format!("{}={}", k, v));
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
    .current_dir(dir)
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
) -> Result<()> {
  ensure_terraform_installed()?;
  let file = file
    .as_ref()
    .canonicalize()
    .context("Failed to canonicalize Terraform path")?;
  let dir = file
    .parent()
    .context("Cannot determine Terraform directory")?;

  let mut cmd = Command::new("terraform");
  cmd.current_dir(dir).arg("destroy").arg("-auto-approve");
  for (k, v) in vars {
    cmd.arg("-var").arg(format!("{}={}", k, v));
  }
  let status = cmd
    .status()
    .context("Failed to execute `terraform destroy`")?;
  if !status.success() {
    bail!("`terraform destroy` failed with exit code {}", status);
  }
  Ok(())
}
