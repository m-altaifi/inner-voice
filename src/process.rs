//! Bounded, hidden child processes. No shell receives transcript or query text.
use anyhow::{Context, Result, bail};
use crossbeam_channel::Receiver;
use std::io::{Read, Write};
use std::os::windows::{io::AsRawHandle, process::CommandExt};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::*;
use windows::core::PCWSTR;

const OUTPUT_LIMIT: usize = 1_048_576;
/// How long a pipe thread gets *after* the child has exited. Everything left is
/// the ≤64 KB still sitting in the pipe, which drains at memcpy speed, so this
/// is thousands of times the honest cost and cannot fire on a slow-but-healthy
/// reader. An expiry means something outside the job still holds the pipe.
const DRAIN: Duration = Duration::from_secs(2);
struct Job(HANDLE);
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

impl Job {
    fn new() -> Result<Self> {
        unsafe {
            let job = Self(CreateJobObjectW(None, PCWSTR::null())?);
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &info as *const _ as _,
                std::mem::size_of_val(&info) as u32,
            )?;
            Ok(job)
        }
    }
}

/// Truncates at `OUTPUT_LIMIT` and keeps draining. Failing the job instead was
/// worse twice over: the cap exists to bound memory, and progress chatter on
/// stderr was killing runs whose stdout held a perfectly good answer. Draining
/// must continue either way or the child blocks on a full pipe.
fn collect(mut reader: impl Read) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0; 8192];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Ok(out);
        }
        if out.len() + n <= OUTPUT_LIMIT {
            out.extend_from_slice(&buf[..n]);
        }
    }
}

/// A result channel, not a `JoinHandle`, because a wait on a pipe thread has to
/// be abandonable: a descendant spawned in the window between `spawn()` and
/// `AssignProcessToJobObject` is outside the job, survives the kill and can hold
/// a pipe open after the child exits — and `join()` on that never returns.
/// A panicking worker drops its sender, so the wait ends either way.
/// ponytail: an expired wait leaks the thread; one job runs at a time, so revisit
/// only if that stops holding.
fn detached<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx
}

pub fn run(
    mut command: Command,
    input: String,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<String> {
    let job = Job::new()?;
    let mut child = command
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting child executable")?;
    if let Err(e) = unsafe { AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle())) } {
        let _ = child.kill();
        let _ = child.wait();
        return Err(e).context("containing child process; refusing an unmanaged job");
    }
    let mut stdin = child.stdin.take().context("child stdin")?;
    let stdout = child.stdout.take().context("child stdout")?;
    let stderr = child.stderr.take().context("child stderr")?;
    let writer = detached(move || stdin.write_all(input.as_bytes()));
    let output = detached(move || collect(stdout));
    let errors = detached(move || collect(stderr));
    let start = Instant::now();
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err(anyhow::anyhow!("job cancelled"));
        }
        if start.elapsed() >= timeout {
            break Err(anyhow::anyhow!(
                "job timed out after {}s",
                timeout.as_secs()
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => break Err(e.into()),
        }
    };
    // Close the job before waiting on pipe readers: a descendant may hold pipes.
    drop(job);
    let _ = child.wait();
    // A cancelled or timed-out job returns without waiting for anything at all:
    // its output is being thrown away regardless, and F9 has to feel immediate.
    let status = result?;
    if !status.success() {
        let err = errors
            .recv_timeout(DRAIN)
            .context("reading child stderr")??;
        let detail = String::from_utf8_lossy(&err);
        bail!(
            "child exited {status}: {}",
            detail.chars().take(1000).collect::<String>()
        );
    }
    let written = writer.recv_timeout(DRAIN).context("writing child stdin")?;
    let out = output
        .recv_timeout(DRAIN)
        .context("reading child stdout")??;
    // A child that answers without draining stdin closes the pipe under the
    // writer; on a prompt past the ~64 KB pipe buffer that surfaces as
    // BrokenPipe. It exited 0 — the answer is real, the write is not news.
    if let Err(e) = written
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        return Err(e).context("sending job input");
    }
    String::from_utf8(out).context("child output is not UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shell(script: &str) -> Command {
        let mut command = Command::new(
            std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
        );
        command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        command
    }
    #[test]
    fn pipes_input_without_shell_interpolation() {
        let input = "literal $(not-a-command) & data";
        let output = run(
            shell("[Console]::Write([Console]::In.ReadToEnd())"),
            input.into(),
            Duration::from_secs(10),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(output, input);
    }
    #[test]
    fn enforces_deadline_and_reports_nonzero_exit() {
        let start = Instant::now();
        let result = run(
            shell("Start-Sleep -Seconds 30"),
            String::new(),
            Duration::from_millis(300),
            &AtomicBool::new(false),
        );
        assert!(result.unwrap_err().to_string().contains("timed out"));
        // The deadline has to be real: nothing is waited on once it fires.
        assert!(start.elapsed() < Duration::from_secs(10));
        assert!(
            run(
                shell("exit 7"),
                String::new(),
                Duration::from_secs(10),
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .to_string()
            .contains("7")
        );
    }
    #[test]
    fn a_wedged_pipe_thread_is_abandoned_not_waited_on() {
        // Stands in for a descendant outside the job still holding a pipe after
        // the child exited: the wait expires instead of taking the caller with
        // it. A real one cannot be staged — anything we spawn is inside the job.
        let stuck = detached(|| std::thread::sleep(Duration::from_secs(60)));
        let start = Instant::now();
        assert!(stuck.recv_timeout(DRAIN).is_err());
        assert!(start.elapsed() < DRAIN + Duration::from_secs(1));
        // A dead worker ends the wait too, which is why it needs no panic case.
        let dead = detached(|| panic!("worker died"));
        let start = Instant::now();
        assert!(dead.recv_timeout(DRAIN).is_err());
        assert!(start.elapsed() < DRAIN);
    }
    #[test]
    fn oversize_output_truncates_instead_of_failing() {
        // Progress chatter on stderr must not cost us the answer on stdout.
        let output = run(
            shell(
                "$s = 'x' * 100000; 1..12 | ForEach-Object { [Console]::Error.Write($s) }; [Console]::Write('ok')",
            ),
            String::new(),
            Duration::from_secs(60),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(output, "ok");
        // Past the cap stdout is cut, not the job.
        let output = run(
            shell("$s = 'x' * 100000; 1..15 | ForEach-Object { [Console]::Write($s) }"),
            String::new(),
            Duration::from_secs(60),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(output.len() > 900_000 && output.len() <= OUTPUT_LIMIT);
    }
    #[test]
    fn keeps_the_answer_when_the_child_ignores_stdin() {
        // A prompt past the ~64 KB pipe buffer breaks the writer's pipe when the
        // child exits without draining it. Exit 0 means the answer stands.
        let output = run(
            shell("[Console]::Write('done')"),
            "x".repeat(200_000),
            Duration::from_secs(60),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(output, "done");
    }
}
