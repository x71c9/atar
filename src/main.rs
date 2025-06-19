use anyhow::{bail, Context, Result};
use atar::{deploy as lib_deploy, undeploy as lib_undeploy};
use signal_hook::{
  consts::signal::{SIGINT, SIGTERM},
  iterator::Signals,
};
use std::panic;
use std::{
  collections::HashMap, env, path::PathBuf, process, sync::mpsc, thread,
};

fn main() {
  run().unwrap_or_else(|err| {
    eprintln!("Error: {}", err);
    process::exit(1);
  })
}

fn run() -> Result<()> {
  let mut args: Vec<String> = env::args().collect();
  let debug = args.iter().any(|a| a == "--debug");
  args.retain(|a| a != "--debug");
  if args.len() <= 1 || args[1] == "-h" || args[1] == "--help" {
    print_help();
    return Ok(());
  }
  if args[1] == "--version" {
    println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    return Ok(());
  }
  if args[1] == "deploy" {
    if args.len() >= 3 && (args[2] == "-h" || args[2] == "--help") {
      print_deploy_help();
      return Ok(());
    }
    let mut terraform_file_path: Option<PathBuf> = None;
    let mut vars: HashMap<String, String> = HashMap::new();
    let mut i = 2;
    while i < args.len() {
      match args[i].as_str() {
        "--terraform-path" | "-t" => {
          i += 1;
          if i >= args.len() {
            bail!("--terraform-path requires a path");
          }
          terraform_file_path = Some(PathBuf::from(&args[i]));
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
    let tf_file_path =
      terraform_file_path.context("`--terraform-path` argument is required")?;
    return run_deploy(tf_file_path, vars, debug);
  }
  if args[1] == "undeploy" {
    if args.len() >= 3 && (args[2] == "-h" || args[2] == "--help") {
      print_undeploy_help();
      return Ok(());
    }
    let mut terraform_file_path: Option<PathBuf> = None;
    let mut vars: HashMap<String, String> = HashMap::new();
    let mut i = 2;
    while i < args.len() {
      match args[i].as_str() {
        "--terraform-path" | "-t" => {
          i += 1;
          if i >= args.len() {
            bail!("--terraform-path requires a path");
          }
          terraform_file_path = Some(PathBuf::from(&args[i]));
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
    let tf_file_path =
      terraform_file_path.context("`--terraform-path` argument is required")?;
    return run_undeploy(tf_file_path, vars, debug);
  }
  eprintln!("Unknown command: {}", args[1]);
  print_help();
  process::exit(1);
}

fn print_help() {
  println!(
    "{} {}\n{}\n\n\
     USAGE:\n\n\
     atar [--debug] deploy --terraform-path <PATH> [--<var> <value> ...]\n\n\
     If undeploy fails when exiting, run:\n\n\
     atar [--debug] undeploy --terraform-path <PATH> [--<var> <value> ...]\n\n\
     For help on the `deploy` subcommand, run:\natar deploy --help\n\n\
     For help on the `undeploy` subcommand, run:\natar undeploy --help",
    env!("CARGO_PKG_NAME"),
    env!("CARGO_PKG_VERSION"),
    env!("CARGO_PKG_DESCRIPTION"),
  );
}

fn print_deploy_help() {
  println!(
        "atar deploy\n\n\
         Deploys a Terraform module, waits until interrupted, then destroys it.\n\n\
         USAGE:\n  atar deploy --terraform <PATH> [--<var> <value> ...]\n\n\
         FLAGS:\n  --terraform <PATH>    Path to Terraform `main.tf` file\n  \
         --<var> <value>       Terraform variable\n"
    );
}

fn print_undeploy_help() {
  println!(
    "atar undeploy\n\n\
         Destroys an existing Terraform deployment.\n\n\
         USAGE:\n  atar undeploy --terraform <PATH> [--<var> <value> ...]\n\n\
         FLAGS:\n  --terraform <PATH>    Path to Terraform `main.tf` file\n  \
         --<var> <value>       Terraform variable\n"
  );
}

fn run_deploy(
  file: PathBuf,
  vars: HashMap<String, String>,
  debug: bool,
) -> Result<()> {
  // Log init/apply steps with file path and each variable on its own line
  // Print variables once, then show placeholders for init/apply
  println!("Variables:");
  println!("  path: {}", file.display());
  for (k, v) in &vars {
    println!("  {}: {}", k, v);
  }

  let outputs = lib_deploy(&file, &vars, debug)?;
  if !outputs.is_empty() {
    println!("*************************** Outputs **************************");
    for (k, v) in outputs {
      println!("{}: {}", k, v);
    }
    println!("**************************************************************");
  }
  // Setup cleanup guard and panic hook (unwinding) after resources are deployed
  let guard = DestroyGuard {
    file: file.clone(),
    vars: vars.clone(),
    debug,
  };
  {
    let fh = file.clone();
    let vh = vars.clone();
    let dbg = debug;
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
      eprintln!("panic: {:?}, cleaning up Terraform...", info);
      if let Err(err) = lib_undeploy(&fh, &vh, dbg) {
        eprintln!("cleanup after panic failed: {}", err);
      }
      previous(info);
    }));
  }
  let (tx, rx) = mpsc::channel();
  let mut signals =
    Signals::new(&[SIGINT, SIGTERM]).context("Failed to set signal handler")?;
  thread::spawn(move || {
    for _ in signals.forever() {
      let _ = tx.send(());
      break;
    }
  });
  println!(
    "Resources deployed.\n\nPress Ctrl+C or send SIGTERM to destroy and exit."
  );
  let _ = rx.recv();
  println!("\nSignal received: starting Terraform destroy...");
  drop(guard);
  Ok(())
}

fn run_undeploy(
  file: PathBuf,
  vars: HashMap<String, String>,
  debug: bool,
) -> Result<()> {
  // Print variables once, then placeholder for destroy
  println!("Variables:");
  println!("  path: {}", file.display());
  for (k, v) in &vars {
    println!("  {}: {}", k, v);
  }

  lib_undeploy(&file, &vars, debug)?;
  Ok(())
}

struct DestroyGuard {
  file: PathBuf,
  vars: HashMap<String, String>,
  debug: bool,
}

impl Drop for DestroyGuard {
  fn drop(&mut self) {
    lib_undeploy(&self.file, &self.vars, self.debug).unwrap_or_else(|err| {
      eprintln!("Failed to destroy Terraform resources: {}", err);
    });
  }
}
