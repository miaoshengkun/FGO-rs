#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use clap::Parser;
use fgo_rs::gui::{GuiOptions, run};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "FGO-rs desktop")]
struct Args {
    /// Directory containing assets/, profiles/ and scripts/.
    #[arg(long)]
    resources: Option<PathBuf>,
    #[arg(long, default_value = "adb")]
    adb: PathBuf,
    /// Writable directory for run logs and recorded frames.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Select an additional script outside the resource directory.
    #[arg(long)]
    script: Option<PathBuf>,
    #[arg(long)]
    profile: Option<PathBuf>,
    /// Offline source; device input is disabled in this mode.
    #[arg(long)]
    replay: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    run(GuiOptions {
        resources: args.resources,
        adb: args.adb,
        output: args.output,
        script: args.script,
        profile: args.profile,
        replay: args.replay,
    })
}
