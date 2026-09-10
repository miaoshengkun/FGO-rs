use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use fgo_rs::{
    adb::Adb,
    control::Control,
    device::{AdbDevice, Device, ReplayDevice},
    farming::Apple,
    runtime::{Host, Runtime},
    vision::{self, Rect, Vision},
};
use std::{
    io::{self, BufRead, BufWriter},
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
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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
        /// Print the complete CN battle state instead of template matches.
        #[arg(long, conflicts_with = "name")]
        battle: bool,
        #[arg(long,conflicts_with_all=["name","battle"])]
        farm: bool,
        #[arg(long, value_parser = parse_rect)]
        viewport: Option<Rect>,
    },
    /// Check Rhai syntax only (does not validate host API calls).
    Check { script: PathBuf },
    /// Execute Rhai. Touches are recorded only unless --execute is supplied.
    Run {
        script: PathBuf,
        #[arg(long, required_unless_present = "replay", conflicts_with = "replay")]
        serial: Option<String>,
        /// A PNG, or directory of PNGs; each capture consumes the next frame.
        #[arg(long, conflicts_with = "execute")]
        replay: Option<PathBuf>,
        #[arg(long, requires = "serial")]
        execute: bool,
        #[arg(long, default_value = "assets/templates.json")]
        templates: PathBuf,
        #[arg(long, value_parser = parse_rect)]
        viewport: Option<Rect>,
        #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(1..=86400))]
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

fn parent_dir(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let max_seconds = if let Commands::Run { max_seconds, .. } = &cli.command {
        *max_seconds
    } else {
        120
    };
    let control = Control::new(Duration::from_secs(max_seconds));
    let stop = control.clone();
    ctrlc::set_handler(move || stop.stop()).context("install Ctrl-C handler")?;
    let adb = Adb {
        executable: cli.adb,
        timeout: Duration::from_secs(cli.adb_timeout),
    };
    match cli.command {
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
            battle,
            farm,
            viewport,
        } => {
            let (frame, _) = vision::normalize(&image::open(&image)?.to_rgba8(), viewport)?;
            let vision = Vision::load(&templates)?;
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
            execute,
            templates,
            viewport,
            max_operations,
            trace,
            frames,
            battles,
            apples,
            apple_kind,
            ..
        } => {
            let source = std::fs::read_to_string(&script)
                .with_context(|| format!("read {}", script.display()))?;
            Runtime::check(&source)?;
            let vision = Vision::load(&templates)?;
            let device: Box<dyn Device> = if let Some(path) = replay {
                Box::new(ReplayDevice::new(&path)?)
            } else {
                Box::new(AdbDevice {
                    adb,
                    serial: serial.context("missing --serial")?,
                })
            };
            parent_dir(&trace)?;
            let output = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&trace)?;
            let mut host = Host::new(
                device,
                vision,
                viewport,
                execute,
                Some(Box::new(BufWriter::new(output))),
                control.clone(),
            );
            if let Some(frames) = frames {
                host.record_frames(frames)?;
            }
            host.configure_farm(battles, apples, Apple::parse(&apple_kind)?);
            let runtime = Runtime::new(host, max_operations);
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
            if let Err(error) = runtime.run(&source) {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_millis();
                let diagnostic = trace.with_file_name(format!("failure-{stamp}.png"));
                if runtime.save_last_frame(&diagnostic).is_ok() {
                    eprintln!("Last observed frame: {}", diagnostic.display());
                }
                return Err(error).context("script failed");
            }
        }
    }
    Ok(())
}
