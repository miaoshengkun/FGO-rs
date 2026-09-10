use crate::control::Control;
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::{
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const OUTPUT_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct Adb {
    pub executable: PathBuf,
    pub timeout: Duration,
}

#[derive(Debug, Serialize)]
pub struct DeviceInfo {
    pub serial: String,
    pub state: String,
    pub details: String,
}

impl Adb {
    pub fn command(
        &self,
        serial: Option<&str>,
        args: &[&str],
        control: &Control,
    ) -> Result<Vec<u8>> {
        control.checkpoint()?;
        let mut command = Command::new(&self.executable);
        if let Some(serial) = serial {
            ensure!(
                !serial.is_empty() && !serial.starts_with('-'),
                "invalid device serial"
            );
            command.args(["-s", serial]);
        }
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("start ADB at {}", self.executable.display()))?;
        let stdout = child.stdout.take().context("missing ADB stdout")?;
        let stderr = child.stderr.take().context("missing ADB stderr")?;
        // Drain both pipes concurrently so a full PNG/stdout pipe cannot deadlock wait().
        let out = thread::spawn(move || read_limited(stdout));
        let err = thread::spawn(move || read_limited(stderr));
        let begin = Instant::now();
        let status = loop {
            let check = control.check_cancel().and_then(|_| {
                ensure!(
                    begin.elapsed() < self.timeout,
                    "ADB command timed out after {:?}",
                    self.timeout
                );
                Ok(())
            });
            if let Err(error) = check {
                break Err(error);
            }
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => thread::sleep(Duration::from_millis(20)),
                Err(error) => break Err(error.into()),
            }
        };
        if status.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let stdout = out
            .join()
            .map_err(|_| anyhow::anyhow!("ADB stdout reader panicked"))?;
        let stderr = err
            .join()
            .map_err(|_| anyhow::anyhow!("ADB stderr reader panicked"))?;
        let status = status?;
        let stdout = stdout?;
        let stderr = stderr?;
        if !status.success() {
            bail!(
                "ADB failed ({status}): {}",
                String::from_utf8_lossy(&stderr).trim()
            );
        }
        control.check_cancel()?;
        Ok(stdout)
    }

    pub fn devices(&self, control: &Control) -> Result<Vec<DeviceInfo>> {
        let output = self.command(None, &["devices", "-l"], control)?;
        let text = String::from_utf8(output).context("ADB devices returned invalid UTF-8")?;
        Ok(parse_devices(&text))
    }
}

fn read_limited(reader: impl Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(OUTPUT_LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= OUTPUT_LIMIT,
        "ADB output exceeded 64 MiB"
    );
    Ok(bytes)
}

pub fn parse_devices(text: &str) -> Vec<DeviceInfo> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with("List of devices") || line.starts_with('*') {
                return None;
            }
            let mut fields = line.split_whitespace();
            Some(DeviceInfo {
                serial: fields.next()?.into(),
                state: fields.next()?.into(),
                details: fields.collect::<Vec<_>>().join(" "),
            })
        })
        .collect()
}
