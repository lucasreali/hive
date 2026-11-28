//! Service baselines (29.8, `docs/review/performance.md`): the `hive` service of this build, in a
//! throwaway `HOME`/`XDG_*`/`HIVE_HOME` (never the real ones), driven over its socket the way the
//! app drives it. No process named `claude` is ever started.
//!
//! `cargo bench -p hive --bench service_perf -- [scenario] [args]` (no scenario: every one with its
//! defaults). Each prints a JSON line.
//! - `startup [runs]` (10): `hive bridge` started in a fresh folder until `welcome`, as the app's
//!   first launch: the bridge, the service it starts and the handshake.
//! - `idle [secs]` (30): the service with the app connected and no terminal.
//! - `shells <n> [secs]` (10, 30): `n` terminals at their prompt.
//! - `stream <n> [secs]` (10, 30): `n` terminals each printing about 25 KB/s of coloured lines
//!   (an agent's output is about 23 KB/s, `src/transport/replay.ts`), every byte acknowledged at
//!   once, as an app that keeps up does.
//! - `flood <n> [secs]` (1, 10): `n` terminals printing as fast as flow control lets them.
//! - `echo <n> [runs]` (0, 300): a key sent to a terminal running `cat` until its echo is back,
//!   with `n` other terminals streaming meanwhile.
//! - `refresh <worktrees> [runs]` (20, 30): `list_projects` until `projects`, for one repository
//!   with that many linked worktrees (each with its health: `git status`, `git diff`, …).
//!
//! CPU is the service process's own (user + system time over wall time, 100% = one core), from
//! `ps` (macOS) or `/proc` (Linux); memory its RSS, and on macOS also `footprint`.

#[cfg(unix)]
use std::error::Error;
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Command;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use futures_util::{SinkExt, StreamExt};
#[cfg(unix)]
use hive_protocol::{Control, Frame, FrameCodec, FrameType, Role};
#[cfg(unix)]
use tokio::net::UnixStream;
#[cfg(unix)]
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
#[cfg(unix)]
use tokio_util::codec::{FramedRead, FramedWrite};

#[cfg(unix)]
type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Time for the terminals' programs to start before a measurement.
#[cfg(unix)]
const WARM_UP: Duration = Duration::from_secs(3);

/// The throwaway environment: one temporary folder holding `HOME`, the runtime and Hive's folder.
#[cfg(unix)]
struct Env {
    dir: tempfile::TempDir,
}

#[cfg(unix)]
impl Env {
    fn new() -> Result<Self> {
        let env = Self {
            dir: tempfile::tempdir()?,
        };
        for sub in ["home", "run", "data", "config", "work"] {
            std::fs::create_dir(env.path(sub))?;
        }
        // As the integration tests do: fish's first start would generate completions from every
        // man page in the background, seconds of a core per terminal that a user pays once.
        std::fs::create_dir_all(env.path("cache/fish/generated_completions"))?;
        Ok(env)
    }

    fn path(&self, sub: &str) -> PathBuf {
        self.dir.path().join(sub)
    }

    fn hive(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hive"));
        for (var, sub) in [
            ("HOME", "home"),
            ("XDG_RUNTIME_DIR", "run"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_CACHE_HOME", "cache"),
            ("HIVE_HOME", "data/hive"),
        ] {
            cmd.env(var, self.path(sub));
        }
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("HIVE_") && key != "HIVE_HOME" {
                cmd.env_remove(key);
            }
        }
        cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CEILING_DIRECTORIES", self.dir.path())
            .env_remove("CLAUDE_CONFIG_DIR")
            .env(hive::power::OFF, "1")
            .env("SHELL", "fish");
        cmd
    }

    fn socket(&self) -> PathBuf {
        self.path("run/hive/hive.sock")
    }
}

/// A `hive daemon` with the app's connection.
#[cfg(unix)]
struct Service {
    child: std::process::Child,
    from: FramedRead<OwnedReadHalf, FrameCodec>,
    to: FramedWrite<OwnedWriteHalf, FrameCodec>,
    /// Terminal output received, in bytes.
    received: u64,
}

#[cfg(unix)]
impl Service {
    async fn start(env: &Env) -> Result<Self> {
        let child = env.hive().arg("daemon").spawn()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            match UnixStream::connect(env.socket()).await {
                Ok(stream) => break stream,
                Err(_) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(e) => return Err(e.into()),
            }
        };
        let (read, write) = stream.into_split();
        let mut service = Self {
            child,
            from: FramedRead::new(read, FrameCodec),
            to: FramedWrite::new(write, FrameCodec),
            received: 0,
        };
        service
            .send(Frame::control(0, &Control::hello(Role::App, hive::VERSION)))
            .await?;
        service
            .until(|f| matches!(f.to_control(), Ok(Control::Welcome { .. })))
            .await?;
        Ok(service)
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    async fn send(&mut self, frame: Frame) -> Result<()> {
        Ok(self.to.send(frame).await?)
    }

    /// The next frame; terminal output is counted and acknowledged at once, as an app that keeps up
    /// acknowledges it.
    async fn next(&mut self) -> Result<Frame> {
        let frame = self.from.next().await.ok_or("service closed")??;
        if frame.kind == FrameType::Terminal {
            let bytes = u32::try_from(frame.payload.len())?;
            self.received += u64::from(bytes);
            let ack = Frame::control(frame.channel, &Control::Ack { bytes });
            self.send(ack).await?;
            // fish 4 asks for the primary device attributes and waits for the answer before
            // reading input: answer as xterm.js does.
            let query = b"\x1b[0c";
            if frame.payload.windows(query.len()).any(|w| w == query) {
                self.send(Frame::terminal(frame.channel, &b"\x1b[?1;2c"[..]))
                    .await?;
            }
        }
        Ok(frame)
    }

    /// Reads until a frame matches, within 60 s.
    async fn until(&mut self, mut matches: impl FnMut(&Frame) -> bool) -> Result<Frame> {
        let read = async {
            loop {
                let frame = self.next().await?;
                if matches(&frame) {
                    return Ok(frame);
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(60), read)
            .await
            .map_err(|_| "timed out waiting for the service")?
    }

    /// Reads (and acknowledges) everything for `time`.
    async fn drain(&mut self, time: Duration) -> Result<()> {
        let end = tokio::time::Instant::now() + time;
        loop {
            tokio::select! {
                frame = self.next() => { frame?; }
                () = tokio::time::sleep_until(end) => return Ok(()),
            }
        }
    }

    /// Opens terminals on channels `first..first + n` in `cwd` and waits for each one's prompt.
    async fn open(&mut self, first: u32, n: u32, cwd: &Path) -> Result<()> {
        for channel in first..first + n {
            let open = Control::OpenTerminal {
                cwd: cwd.display().to_string(),
                cols: 120,
                rows: 40,
                account: None,
            };
            self.send(Frame::control(channel, &open)).await?;
        }
        let mut waiting: Vec<u32> = (first..first + n).collect();
        while !waiting.is_empty() {
            let frame = self
                .until(|f| f.kind == FrameType::Terminal && waiting.contains(&f.channel))
                .await?;
            waiting.retain(|c| *c != frame.channel);
        }
        // The rest of each prompt.
        self.drain(Duration::from_millis(500)).await
    }

    /// Types `line` and Enter into terminal `channel`.
    async fn run(&mut self, channel: u32, line: &str) -> Result<()> {
        self.send(Frame::terminal(channel, format!("{line}\r").into_bytes()))
            .await
    }

    /// Ends the service by closing the app connection (it lives as long as that connection).
    fn stop(self) -> Result<()> {
        let Self {
            mut child,
            from,
            to,
            ..
        } = self;
        drop((from, to));
        let deadline = Instant::now() + Duration::from_secs(15);
        while child.try_wait()?.is_none() {
            if Instant::now() > deadline {
                child.kill()?;
                return Err("the service did not end".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }
}

/// A process's CPU time (user + system) and resident memory.
#[cfg(unix)]
struct Usage {
    cpu: Duration,
    rss: u64,
}

#[cfg(target_os = "macos")]
fn usage(pid: u32) -> Result<Usage> {
    // `time` is [[dd-]hh:]mm:ss.cc; `rss` is in KiB.
    let out = Command::new("ps")
        .args(["-o", "time=,rss=", "-p", &pid.to_string()])
        .output()?;
    let text = String::from_utf8(out.stdout)?;
    let mut fields = text.split_whitespace();
    let time = fields.next().ok_or("no cpu time from ps")?;
    let rss: u64 = fields.next().ok_or("no rss from ps")?.parse()?;
    let (days, clock) = time.split_once('-').unwrap_or(("0", time));
    let mut secs = days.parse::<f64>()? * 86_400.0;
    let mut unit = 1.0;
    for part in clock.rsplit(':') {
        secs += part.parse::<f64>()? * unit;
        unit *= 60.0;
    }
    Ok(Usage {
        cpu: Duration::from_secs_f64(secs),
        rss: rss * 1024,
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn usage(pid: u32) -> Result<Usage> {
    // /proc/<pid>/stat: utime and stime are fields 14 and 15, in clock ticks (100 a second on
    // Linux); statm's second field is the resident pages.
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let after = stat.rsplit_once(')').ok_or("bad /proc stat")?.1;
    let fields: Vec<&str> = after.split_whitespace().collect();
    let ticks: u64 = fields.get(11).ok_or("no utime")?.parse::<u64>()?
        + fields.get(12).ok_or("no stime")?.parse::<u64>()?;
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm"))?;
    let pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .ok_or("bad statm")?
        .parse()?;
    Ok(Usage {
        cpu: Duration::from_millis(ticks * 10),
        rss: pages * 4096,
    })
}

/// macOS' physical footprint (what Activity Monitor shows), in bytes; 0 elsewhere.
#[cfg(unix)]
fn footprint(pid: u32) -> u64 {
    if !cfg!(target_os = "macos") {
        return 0;
    }
    let Ok(out) = Command::new("footprint")
        .args(["-f", "bytes", "--noCategories", "-p", &pid.to_string()])
        .output()
    else {
        return 0;
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| line.trim().strip_prefix("phys_footprint:"))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

#[cfg(unix)]
fn mb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_048_576.0)
}

/// Reads (and acknowledges) the service's output for `secs`, sampling its usage once a second.
/// Prints one JSON line for `scenario`.
#[cfg(unix)]
async fn sample(service: &mut Service, scenario: &str, terminals: u32, secs: u64) -> Result<()> {
    let pid = service.pid();
    let start = usage(pid)?;
    let (began, received) = (Instant::now(), service.received);
    let mut peak = start.rss;
    for _ in 0..secs {
        service.drain(Duration::from_secs(1)).await?;
        peak = peak.max(usage(pid)?.rss);
    }
    let end = usage(pid)?;
    let wall = began.elapsed().as_secs_f64();
    let cpu = (end.cpu.saturating_sub(start.cpu)).as_secs_f64() / wall * 100.0;
    let rate = (service.received - received) as f64 / wall / 1024.0;
    println!(
        r#"{{"scenario":"{scenario}","terminals":{terminals},"secs":{wall:.1},"cpu_pct":{cpu:.1},"rss_mb":{},"rss_peak_mb":{},"footprint_mb":{},"output_kib_s":{rate:.0}}}"#,
        mb(end.rss),
        mb(peak),
        mb(footprint(pid)),
    );
    Ok(())
}

/// Writes the streaming programs into `dir`: `stream.sh` prints a 1 KB chunk of coloured lines
/// every 40 ms, `flood.sh` a 64 KB one without pause; both run until their terminal closes.
#[cfg(unix)]
fn programs(dir: &Path) -> Result<()> {
    let mut chunk = String::new();
    for i in 0..12 {
        chunk.push_str(&format!(
            "\x1b[38;2;{};{};200m  ⎿  line {i:02} of an agent's answer, streamed\x1b[0m {}\n",
            100 + i * 10,
            180 - i * 5,
            "·".repeat(20),
        ));
    }
    std::fs::write(dir.join("chunk.txt"), &chunk)?;
    std::fs::write(dir.join("big.txt"), chunk.repeat(64))?;
    let stream = format!(
        "while :; do cat '{0}/chunk.txt'; sleep 0.04; done\n",
        dir.display()
    );
    std::fs::write(dir.join("stream.sh"), stream)?;
    let flood = format!("while :; do cat '{0}/big.txt'; done\n", dir.display());
    std::fs::write(dir.join("flood.sh"), flood)?;
    Ok(())
}

#[cfg(unix)]
async fn terminals(scenario: &str, n: u32, secs: u64) -> Result<()> {
    let env = Env::new()?;
    programs(&env.path("work"))?;
    let mut service = Service::start(&env).await?;
    if n > 0 {
        service.open(1, n, &env.path("home")).await?;
    }
    let program = match scenario {
        "stream" => Some("stream.sh"),
        "flood" => Some("flood.sh"),
        _ => None,
    };
    if let Some(program) = program {
        let line = format!("exec sh '{}'", env.path("work").join(program).display());
        for channel in 1..=n {
            service.run(channel, &line).await?;
        }
    }
    service.drain(WARM_UP).await?;
    sample(&mut service, scenario, n, secs).await?;
    service.stop()
}

#[cfg(unix)]
fn percentiles(name: &str, mut samples: Vec<Duration>) -> String {
    samples.sort();
    let at = |p: usize| {
        let i = (samples.len() * p / 100).min(samples.len().saturating_sub(1));
        samples.get(i).map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    };
    format!(
        r#""{name}_ms":{{"p50":{:.2},"p95":{:.2},"p99":{:.2},"max":{:.2}}}"#,
        at(50),
        at(95),
        at(99),
        at(100)
    )
}

#[cfg(unix)]
async fn echo(streaming: u32, runs: usize) -> Result<()> {
    let env = Env::new()?;
    programs(&env.path("work"))?;
    let mut service = Service::start(&env).await?;
    service.open(1, streaming + 1, &env.path("home")).await?;
    let line = format!("exec sh '{}'", env.path("work").join("stream.sh").display());
    for channel in 2..=streaming + 1 {
        service.run(channel, &line).await?;
    }
    // `cat` in the line discipline's canonical mode: the kernel echoes each key at once.
    service.run(1, "exec cat").await?;
    service.drain(WARM_UP).await?;
    let mut samples = Vec::with_capacity(runs);
    for run in 0..runs {
        // A line ends every 200 keys, within the line discipline's 1024-byte limit.
        if run % 200 == 199 {
            service.run(1, "").await?;
            service.drain(Duration::from_millis(100)).await?;
        }
        let start = Instant::now();
        service.send(Frame::terminal(1, &b"k"[..])).await?;
        service
            .until(|f| f.kind == FrameType::Terminal && f.channel == 1 && f.payload.contains(&b'k'))
            .await?;
        samples.push(start.elapsed());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    println!(
        r#"{{"scenario":"echo","streaming":{streaming},"runs":{runs},{}}}"#,
        percentiles("echo", samples)
    );
    service.stop()
}

#[cfg(unix)]
fn git(dir: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=Bench",
            "-c",
            "user.email=bench@example.com",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("git {args:?} failed").into())
    }
}

/// A repository of 500 files in 25 folders, one commit, with `worktrees` linked worktrees under
/// `.claude/worktrees/`, one commit ahead each; every third one has two files changed.
#[cfg(unix)]
fn repository(at: &Path, worktrees: u32) -> Result<()> {
    std::fs::create_dir(at)?;
    git(at, &["init", "-q", "-b", "main"])?;
    for d in 0..25 {
        let dir = at.join(format!("src/mod{d:02}"));
        std::fs::create_dir_all(&dir)?;
        for f in 0..20 {
            let body: String = (0..40).map(|l| format!("line {l} of file {f}\n")).collect();
            std::fs::write(dir.join(format!("file{f:02}.ts")), body)?;
        }
    }
    std::fs::write(at.join(".gitignore"), ".claude/\n")?;
    git(at, &["add", "-A"])?;
    git(at, &["commit", "-q", "-m", "initial"])?;
    for w in 0..worktrees {
        let name = format!("task-{w:02}");
        let path = at.join(".claude/worktrees").join(&name);
        let path_str = path.display().to_string();
        git(at, &["worktree", "add", "-q", "-b", &name, &path_str])?;
        std::fs::write(path.join(format!("src/mod00/new{w}.ts")), "export {};\n")?;
        git(&path, &["add", "-A"])?;
        git(&path, &["commit", "-q", "-m", "work"])?;
        if w % 3 == 0 {
            std::fs::write(path.join("src/mod01/file01.ts"), "changed\n")?;
            std::fs::write(path.join("notes.md"), "untracked\nfile\n")?;
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn refresh(worktrees: u32, runs: usize) -> Result<()> {
    let env = Env::new()?;
    let repo = env.path("work/repo");
    repository(&repo, worktrees)?;
    let mut service = Service::start(&env).await?;
    let add = Control::AddProject {
        path: repo.display().to_string(),
    };
    service.send(Frame::control(0, &add)).await?;
    let answer = service
        .until(|f| {
            matches!(
                f.to_control(),
                Ok(Control::ProjectAdded { .. } | Control::AddProjectFailed { .. })
            )
        })
        .await?;
    if let Ok(Control::AddProjectFailed { message, .. }) = answer.to_control() {
        return Err(message.into());
    }
    let mut samples = Vec::with_capacity(runs);
    let mut listed = 0;
    for _ in 0..runs {
        let start = Instant::now();
        service
            .send(Frame::control(0, &Control::ListProjects))
            .await?;
        let frame = service
            .until(|f| matches!(f.to_control(), Ok(Control::Projects { .. })))
            .await?;
        samples.push(start.elapsed());
        if let Ok(Control::Projects { projects }) = frame.to_control() {
            listed = projects.iter().map(|p| p.worktrees.len()).sum::<usize>();
        }
    }
    println!(
        r#"{{"scenario":"refresh","worktrees":{listed},"runs":{runs},{}}}"#,
        percentiles("refresh", samples)
    );
    service.stop()
}

/// The app's first launch: `hive bridge` in a fresh folder until `welcome`.
#[cfg(unix)]
async fn startup(runs: usize) -> Result<()> {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let env = Env::new()?;
        let start = Instant::now();
        let mut bridge = tokio::process::Command::from(env.hive())
            .arg("bridge")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let mut to = FramedWrite::new(bridge.stdin.take().ok_or("no stdin")?, FrameCodec);
        let mut from = FramedRead::new(bridge.stdout.take().ok_or("no stdout")?, FrameCodec);
        to.send(Frame::control(0, &Control::hello(Role::App, hive::VERSION)))
            .await?;
        let frame = from.next().await.ok_or("bridge closed")??;
        if !matches!(frame.to_control(), Ok(Control::Welcome { .. })) {
            return Err("no welcome".into());
        }
        samples.push(start.elapsed());
        drop((to, from));
        bridge.wait().await?;
        // The service ends with the app connection; wait for it to let go of its socket.
        let deadline = Instant::now() + Duration::from_secs(10);
        while env.socket().exists() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    println!(
        r#"{{"scenario":"startup","runs":{runs},{}}}"#,
        percentiles("welcome", samples)
    );
    Ok(())
}

#[cfg(unix)]
async fn scenario(args: &[String]) -> Result<()> {
    let num = |i: usize, default: u64| -> Result<u64> {
        args.get(i).map_or(Ok(default), |a| Ok(a.parse()?))
    };
    let name = args.first().map(String::as_str);
    match name {
        Some("startup") => startup(usize::try_from(num(1, 10)?)?).await,
        Some("idle") => terminals("idle", 0, num(1, 30)?).await,
        Some(s @ ("shells" | "stream" | "flood")) => {
            let n = u32::try_from(num(1, if s == "flood" { 1 } else { 10 })?)?;
            terminals(s, n, num(2, if s == "flood" { 10 } else { 30 })?).await
        }
        Some("echo") => echo(u32::try_from(num(1, 0)?)?, usize::try_from(num(2, 300)?)?).await,
        Some("refresh") => {
            refresh(u32::try_from(num(1, 20)?)?, usize::try_from(num(2, 30)?)?).await
        }
        Some(other) => Err(format!("unknown scenario {other}").into()),
        None => {
            startup(10).await?;
            terminals("idle", 0, 30).await?;
            terminals("shells", 10, 30).await?;
            for n in [10, 30] {
                terminals("stream", n, 30).await?;
            }
            terminals("flood", 1, 10).await?;
            terminals("flood", 10, 10).await?;
            for n in [0, 10, 30] {
                echo(n, 300).await?;
            }
            refresh(20, 30).await
        }
    }
}

#[cfg(unix)]
#[tokio::main]
async fn main() -> Result<()> {
    // `cargo bench` passes `--bench`; the rest is the scenario.
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with("--"))
        .collect();
    scenario(&args).await
}

/// The service does not run on Windows in these measurements (12.5.2).
#[cfg(windows)]
fn main() {}
