use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use fgo_rs::{
    adb::Adb,
    catalog::{AtlasClient, Region},
    control::Control,
    device::{AdbDevice, FrameSource},
    farming::Apple,
    profile::PartyProfile,
    replay::ReplayComparisonMode,
    runner::{self, RecognitionConfig, RunConfig, RunHooks, configure_party_vision, parent_dir},
    runtime::Runtime,
    vision::{self, Rect},
};
use std::{
    io::{self, BufRead},
    path::PathBuf,
    time::Duration,
};

#[derive(Parser)]
#[command(version, about = "FGO Rust + Rhai automation core")]
struct Cli {
    /// Official platform-tools adb executable (or an executable on PATH).
    #[arg(long, global = true, default_value = "adb")]
    adb: PathBuf,
    #[arg(long, global = true, default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..=300))]
    adb_timeout: u64,
    /// Use MaaFramework v5.14.2 for template recognition (build with --features maa).
    #[arg(long, global = true)]
    maa_library: Option<PathBuf>,
    /// Additional MaaFramework resource bundles, layered after assets/maa.
    #[arg(long, global = true, requires = "maa_library")]
    maa_resource: Vec<PathBuf>,
    /// Directory containing det.onnx, rec.onnx and keys.txt for native OCR.
    #[arg(long, global = true, requires = "maa_library")]
    maa_ocr_model: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Update the local servant catalog from Atlas Academy.
    SyncData {
        /// Game data region: JP, NA, CN, KR, or TW.
        #[arg(long, default_value_t = Region::Cn)]
        region: Region,
        /// Root directory for versioned Atlas Academy snapshots.
        #[arg(long, default_value = "data/atlas")]
        data_dir: PathBuf,
        /// Also cache compact details for fixed members and support candidates.
        #[arg(long)]
        profile: Option<PathBuf>,
    },
    /// List connected devices, including offline/unauthorized devices.
    Devices,
    /// Connect to a network Android device. USB devices do not need this command.
    Connect { address: String },
    /// Capture a normalized screenshot. Does not send touch input.
    Screenshot {
        #[arg(long)]
        serial: String,
        #[arg(long, default_value = "screenshot.png")]
        output: PathBuf,
        #[arg(long, value_parser = parse_rect)]
        viewport: Option<Rect>,
    },
    /// Inspect a PNG against the template manifest without a device.
    Inspect {
        image: PathBuf,
        #[arg(long, default_value = "assets/templates.json")]
        templates: PathBuf,
        #[arg(long)]
        name: Option<String>,
        /// Evaluate a MaaFramework resource recognition node without running its actions.
        #[arg(long, conflicts_with_all = ["name", "battle", "farm", "profile"])]
        maa_node: Option<String>,
        /// Print the complete CN battle state instead of template matches.
        #[arg(long, conflicts_with = "name")]
        battle: bool,
        #[arg(long,conflicts_with_all=["name","battle"])]
        farm: bool,
        /// Load local Atlas card identities for battle inspection.
        #[arg(long, requires = "battle")]
        profile: Option<PathBuf>,
        #[arg(long, default_value_t = Region::Cn)]
        region: Region,
        #[arg(long, default_value = "data/atlas")]
        data_dir: PathBuf,
        #[arg(long, value_parser = parse_rect)]
        viewport: Option<Rect>,
    },
    /// Check Rhai syntax and direct host API calls without a device.
    Check { script: PathBuf },
    /// Execute Rhai. Touches are recorded only unless --execute is supplied.
    Run {
        script: PathBuf,
        #[arg(long, required_unless_present = "replay", conflicts_with = "replay")]
        serial: Option<String>,
        /// A PNG, or directory of PNGs; each capture consumes the next frame.
        #[arg(long, conflicts_with = "execute")]
        replay: Option<PathBuf>,
        /// Baseline JSONL from the recorded run; supplies capture timing and expected events.
        #[arg(long, requires = "replay")]
        replay_trace: Option<PathBuf>,
        /// Compare only observations or the complete semantic event stream.
        #[arg(long, requires = "replay_trace")]
        replay_compare: Option<ReplayComparisonMode>,
        /// Write the replay comparison report to this JSON file instead of stdout.
        #[arg(long, requires = "replay_trace")]
        replay_report: Option<PathBuf>,
        #[arg(long, requires = "serial")]
        execute: bool,
        #[arg(long, default_value = "assets/templates.json")]
        templates: PathBuf,
        /// Versioned party markers, support slot, and order-change configuration.
        #[arg(long, default_value = "profiles/rider-v1.json")]
        profile: PathBuf,
        /// Region used to load the Profile's local Atlas data.
        #[arg(long, default_value_t = Region::Cn)]
        region: Region,
        /// Root directory containing versioned local Atlas data.
        #[arg(long, default_value = "data/atlas")]
        data_dir: PathBuf,
        #[arg(long, value_parser = parse_rect)]
        viewport: Option<Rect>,
        /// Overall runtime in seconds, including pauses; 0 disables the deadline.
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=86400))]
        max_seconds: u64,
        #[arg(long, default_value_t = 1_000_000, value_parser = clap::value_parser!(u64).range(1..))]
        max_operations: u64,
        #[arg(long, default_value = "logs/actions.jsonl")]
        trace: PathBuf,
        /// Save each normalized capture to a new directory for replay/debugging.
        #[arg(long)]
        frames: Option<PathBuf>,
        /// Battle limit for farming scripts; 0 runs until AP/apple budget is exhausted.
        #[arg(long, default_value_t = 1)]
        battles: u32,
        #[arg(long, default_value_t = 0)]
        apples: u32,
        #[arg(long,default_value="gold",value_parser=["gold","silver","bronze"])]
        apple_kind: String,
    },
}

fn parse_rect(text: &str) -> std::result::Result<Rect, String> {
    let values = text
        .split(',')
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| "viewport must be x,y,width,height".to_string())?;
    if values.len() != 4 || values[2] == 0 || values[3] == 0 {
        return Err("viewport must be x,y,width,height with positive size".into());
    }
    Ok(Rect {
        x: values[0],
        y: values[1],
        width: values[2],
        height: values[3],
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let max_seconds = if let Commands::Run { max_seconds, .. } = &cli.command {
        *max_seconds
    } else {
        120
    };
    let replaying = matches!(
        &cli.command,
        Commands::Run {
            replay: Some(_),
            ..
        }
    );
    let control = if replaying {
        Control::simulated(runner::execution_limit(max_seconds))
    } else {
        Control::new(runner::execution_limit(max_seconds))
    };
    let stop = control.clone();
    ctrlc::set_handler(move || stop.stop()).context("install Ctrl-C handler")?;
    let adb = Adb {
        executable: cli.adb,
        timeout: Duration::from_secs(cli.adb_timeout),
    };
    let recognition = RecognitionConfig {
        library: cli.maa_library,
        resources: cli.maa_resource,
        ocr_model: cli.maa_ocr_model,
    };
    match cli.command {
        Commands::SyncData {
            region,
            data_dir,
            profile,
        } => {
            let servant_ids = profile
                .map(|path| PartyProfile::load(&path))
                .transpose()?
                .map(|profile| profile.atlas_servant_ids());
            if let Some(ids) = &servant_ids {
                ensure!(!ids.is_empty(), "profile has no Atlas servant IDs");
            }
            let client = AtlasClient::default();
            let catalog = client.sync(&data_dir, region)?;
            if let Some(ids) = servant_ids {
                let details = client.sync_servant_details(&data_dir, region, &ids)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "catalog": catalog,
                        "details": details,
                    }))?
                );
            } else {
                println!("{}", serde_json::to_string_pretty(&catalog)?);
            }
        }
        Commands::Devices => println!("{}", serde_json::to_string_pretty(&adb.devices(&control)?)?),
        Commands::Connect { address } => {
            ensure!(
                !address.starts_with('-')
                    && address.contains(':')
                    && !address.chars().any(char::is_whitespace),
                "address must be host:port"
            );
            let output = adb.command(None, &["connect", &address], &control)?;
            let message = String::from_utf8_lossy(&output);
            ensure!(
                message.contains("connected to"),
                "ADB connection failed: {}",
                message.trim()
            );
            print!("{message}");
        }
        Commands::Screenshot {
            serial,
            output,
            viewport,
        } => {
            let raw = AdbDevice { adb, serial }.capture(&control)?;
            let (frame, _) = vision::normalize(&raw, viewport)?;
            parent_dir(&output)?;
            frame.save(&output)?;
            println!("{}", output.display());
        }
        Commands::Inspect {
            image,
            templates,
            name,
            maa_node,
            battle,
            farm,
            profile,
            region,
            data_dir,
            viewport,
        } => {
            let (frame, _) = vision::normalize(&image::open(&image)?.to_rgba8(), viewport)?;
            let mut vision = recognition.load(&templates)?;
            if let Some(node) = maa_node {
                ensure!(
                    recognition.library.is_some(),
                    "--maa-node requires --maa-library"
                );
                #[cfg(feature = "maa")]
                {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(
                            &vision.recognize_maa_node(&frame, &node, &control)?
                        )?
                    );
                    return Ok(());
                }
                #[cfg(not(feature = "maa"))]
                anyhow::bail!("MaaFramework support is disabled; cannot inspect {node}");
            }
            if let Some(profile_path) = profile {
                let profile = PartyProfile::load(&profile_path)?;
                configure_party_vision(&mut vision, &profile, &data_dir, region)?;
            }
            if farm {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&vision.farm_state(&frame, &control)?)?
                );
                return Ok(());
            }
            if battle {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&vision.battle_state(&frame, &control)?)?
                );
                return Ok(());
            }
            let names = name.map(|n| vec![n]).unwrap_or_else(|| vision.names());
            let mut result = serde_json::Map::new();
            for name in names {
                let found = if name == "support120" {
                    vision.support(&frame, &control)?
                } else {
                    vision.find(&frame, &name, &control)?
                };
                result.insert(name, serde_json::to_value(found)?);
            }
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Commands::Check { script } => {
            Runtime::check(&std::fs::read_to_string(&script)?)?;
            println!("Syntax OK: {}", script.display());
        }
        Commands::Run {
            script,
            serial,
            replay,
            replay_trace,
            replay_compare,
            replay_report,
            execute,
            templates,
            profile,
            region,
            data_dir,
            viewport,
            max_operations,
            trace,
            frames,
            battles,
            apples,
            apple_kind,
            ..
        } => {
            let prepared = runner::prepare(
                RunConfig {
                    fixed_farm: None,
                    adb,
                    script,
                    serial,
                    replay,
                    replay_trace,
                    replay_compare,
                    replay_report,
                    execute,
                    templates,
                    recognition,
                    profile,
                    region,
                    data_dir,
                    viewport,
                    max_seconds,
                    max_operations,
                    trace,
                    frames,
                    battles,
                    apples,
                    apple: Apple::parse(&apple_kind)?,
                },
                control.clone(),
                RunHooks::default(),
            )?;
            eprintln!(
                "Mode: {}. Commands: pause / resume / stop + Enter; Ctrl-C stops.",
                if execute {
                    "device input enabled"
                } else {
                    "record input only"
                }
            );
            let input = control.clone();
            std::thread::spawn(move || {
                for line in io::stdin().lock().lines() {
                    let Ok(line) = line else {
                        break;
                    };
                    match line.trim() {
                        "pause" => input.pause(),
                        "resume" => input.resume(),
                        "stop" => {
                            input.stop();
                            break;
                        }
                        _ => eprintln!("Commands: pause / resume / stop"),
                    }
                }
            });
            prepared.run()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_defaults_to_unlimited_and_accepts_explicit_time_budgets() {
        for (extra, expected) in [
            (vec![], 0),
            (vec!["--max-seconds", "0"], 0),
            (vec!["--max-seconds", "1800"], 1800),
        ] {
            let mut args = vec![
                "fgo-rs",
                "run",
                "scripts/assassin_farm.rhai",
                "--serial",
                "test",
            ];
            args.extend(extra);
            let cli = Cli::try_parse_from(args).unwrap();
            let Commands::Run { max_seconds, .. } = cli.command else {
                panic!("expected run")
            };
            assert_eq!(max_seconds, expected);
        }
    }

    #[test]
    fn sync_data_defaults_to_cn() {
        let cli = Cli::try_parse_from(["fgo-rs", "sync-data"]).unwrap();
        let Commands::SyncData {
            region,
            data_dir,
            profile,
        } = cli.command
        else {
            panic!("expected sync-data command")
        };
        assert_eq!(region, Region::Cn);
        assert_eq!(data_dir, PathBuf::from("data/atlas"));
        assert_eq!(profile, None);
    }

    #[test]
    fn replay_trace_enables_semantic_comparison_options() {
        let cli = Cli::try_parse_from([
            "fgo-rs",
            "run",
            "scripts/rider_battle.rhai",
            "--replay",
            "frames",
            "--replay-trace",
            "baseline.jsonl",
            "--replay-compare",
            "observations",
            "--replay-report",
            "report.json",
        ])
        .unwrap();
        let Commands::Run {
            replay_trace,
            replay_compare,
            replay_report,
            ..
        } = cli.command
        else {
            panic!("expected run command")
        };
        assert_eq!(replay_trace, Some(PathBuf::from("baseline.jsonl")));
        assert_eq!(replay_compare, Some(ReplayComparisonMode::Observations));
        assert_eq!(replay_report, Some(PathBuf::from("report.json")));
    }

    #[test]
    fn sync_data_accepts_case_insensitive_region() {
        let cli = Cli::try_parse_from(["fgo-rs", "sync-data", "--region", "jp"]).unwrap();
        let Commands::SyncData { region, .. } = cli.command else {
            panic!("expected sync-data command")
        };
        assert_eq!(region, Region::Jp);
    }

    #[test]
    fn sync_data_accepts_a_party_profile() {
        let cli =
            Cli::try_parse_from(["fgo-rs", "sync-data", "--profile", "profiles/rider-v1.json"])
                .unwrap();
        let Commands::SyncData { profile, .. } = cli.command else {
            panic!("expected sync-data command")
        };
        assert_eq!(profile, Some(PathBuf::from("profiles/rider-v1.json")));
    }
}
