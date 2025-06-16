use anyhow::{Context, Result, bail};
use ctrlc;
use std::{
  collections::HashMap,
  env,
  path::{Path, PathBuf},
  process::{self, Command, Stdio},
  sync::mpsc,
};

fn main() {
  if let Err(err) = run() {
    eprintln!("Error: {}", err);
    process::exit(1);
  }
}

fn run() -> Result<()> {
  let args: Vec<String> = env::args().collect();
  if args.len() <= 1 || args[1] == "-h" || args[1] == "--help" {
    print_help();
    return Ok(());
  }
  if args[1] == "--version" {
    println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    return Ok(());
  }
  if args[1] != "deploy" {
    eprintln!("Unknown command: {}", args[1]);
    print_help();
    process::exit(1);
  }
  if args.len() >= 3 && (args[2] == "-h" || args[2] == "--help") {
    print_deploy_help();
    return Ok(());
  }

  let mut terraform_file: Option<PathBuf> = None;
  let mut vars: HashMap<String, String> = HashMap::new();
  let mut i = 2;
  while i < args.len() {
    match args[i].as_str() {
      "--terraform" => {
        i += 1;
        if i >= args.len() {
          bail!("--terraform requires a path");
        }
        terraform_file = Some(PathBuf::from(&args[i]));
      }
      arg if arg.starts_with("--") => {
        let key = arg.trim_start_matches("--").to_string();
        i += 1;
        if i >= args.len() {
          bail!("Flag {} requires a value", arg);
        }
        vars.insert(key, args[i].clone());
      }
      other => bail!("Unexpected argument: {}", other),
    }
    i += 1;
  }
  let tf_file = terraform_file.context("`--terraform` argument is required")?;

  run_deploy(tf_file, vars)
}

fn run_deploy(file: PathBuf, vars: HashMap<String, String>) -> Result<()> {
  ensure_terraform_installed()?;
  let file = file
    .canonicalize()
    .context("Failed to canonicalize Terraform path")?;
  let dir = file
    .parent()
    .context("Cannot determine Terraform directory")?;

  println!("Initializing Terraform in {}", dir.display());
  run_command("terraform", &["init"], dir, &vars)?;
  println!("Applying Terraform in {}", dir.display());
  run_command("terraform", &["apply", "-auto-approve"], dir, &vars)?;

  let guard = DestroyGuard {
    dir: dir.to_path_buf(),
    vars: vars.clone(),
  };
  let (tx, rx) = mpsc::channel();
  ctrlc::set_handler(move || {
    let _ = tx.send(());
  })
  .context("Failed to set Ctrl-C handler")?;

  println!("Resources deployed. Press Ctrl+C to destroy and exit.");
  let _ = rx.recv();

  drop(guard);
  Ok(())
}

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

fn run_command(
  cmd: &str,
  args: &[&str],
  dir: &Path,
  vars: &HashMap<String, String>,
) -> Result<()> {
  let mut command = Command::new(cmd);
  command.current_dir(dir).args(args);
  for (k, v) in vars {
    command.arg("-var").arg(format!("{}={}", k, v));
  }
  let status = command
    .status()
    .with_context(|| format!("Failed to execute `{}`", cmd))?;
  if !status.success() {
    bail!("`{}` failed with exit code {}", cmd, status);
  }
  Ok(())
}

struct DestroyGuard {
  dir: PathBuf,
  vars: HashMap<String, String>,
}

impl Drop for DestroyGuard {
  fn drop(&mut self) {
    eprintln!("Destroying Terraform resources in {}", self.dir.display());
    let mut cmd = Command::new("terraform");
    cmd
      .current_dir(&self.dir)
      .arg("destroy")
      .arg("-auto-approve");
    for (k, v) in &self.vars {
      cmd.arg("-var").arg(format!("{}={}", k, v));
    }
    match cmd.status() {
      Ok(status) if status.success() => {
        eprintln!("Resources destroyed.");
      }
      Ok(status) => {
        eprintln!("`terraform destroy` failed with exit code {}", status);
      }
      Err(err) => {
        eprintln!("Failed to execute `terraform destroy`: {}", err);
      }
    }
  }
}

fn print_help() {
  println!(
    "{} {}\n{}\n\nUSAGE:\n  atar deploy --terraform <PATH> [--<var> <value> ...]\n\nFor help on the `deploy` subcommand, run `atar deploy --help`.",
    env!("CARGO_PKG_NAME"),
    env!("CARGO_PKG_VERSION"),
    env!("CARGO_PKG_DESCRIPTION"),
  );
}

fn print_deploy_help() {
  println!(
    "atar deploy\n\nDeploys a Terraform module, waits until interrupted, then destroys it.\n\n    USAGE:\n  atar deploy --terraform <PATH> [--<var> <value> ...]\n\n    FLAGS:\n  --terraform <PATH>    Path to Terraform `main.tf` file\n    --<var> <value>       Terraform variable\n"
  );
}
