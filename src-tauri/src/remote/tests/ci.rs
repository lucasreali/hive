//! A real remote (26.20; the spike's open questions 2 and 5): the app's link code, headless (no
//! WebView), against an `sshd` in a Linux container behind `tc netem`, with 26.13's static
//! `hive`. Only ci.yml's `remote` job builds it (`--features remote-ci`), after setting up the
//! host; it reads where things are from the environment:
//!
//! - `HIVE_CI_SSH`: an `ssh` that reads only the job's own config, keys and `known_hosts`. Its
//!   aliases: `hive-ci` (the host), `hive-unknown` (its key not known), `hive-changed` (another
//!   key known for it) and `hive-darwin` (a user whose `uname` says `Darwin arm64`).
//! - `HIVE_CI_BUILDS`: the folder holding `hive-linux-x86_64`.
//! - `HIVE_CI_RTT_MS`: the round trip `netem` adds.
//! - `HIVE_CI_REPORT`: a file the numbers are appended to (Markdown).

use super::*;

use std::io::Write as _;
use std::time::Instant;

/// Waits on the link's states, generous: a lossy link connects in seconds, and an upload of
/// several MB takes longer.
const LONG: Duration = Duration::from_secs(120);
/// The most a key's echo may take, even during a flood.
const ECHO_WAIT: Duration = Duration::from_secs(30);
/// Keys typed for the echo latency (as the spike, §7.1), and during the flood (§7.2).
const KEYS: usize = 200;
const FLOOD_KEYS: usize = 60;

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set (ci.yml's remote job)"))
}

/// Runs `args` on `alias` through the job's `ssh`; its stdout.
fn ssh(alias: &str, args: &[&str]) -> String {
    let out = std::process::Command::new(var("HIVE_CI_SSH"))
        .args(["-T", "--", alias])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "ssh {alias} {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap()
}

fn report(line: &str) {
    println!("remote-ci: {line}");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(var("HIVE_CI_REPORT"))
        .unwrap();
    writeln!(file, "{line}").unwrap();
}

/// The UI messages up to the first `host_link` in `state` (last), within `wait`.
async fn reach(rx: &mut mpsc::UnboundedReceiver<Value>, state: &str, wait: Duration) -> Vec<Value> {
    let found = tokio::time::timeout(wait, async {
        let mut seen = Vec::new();
        while let Some(message) = rx.recv().await {
            let done = message["type"] == "host_link" && message["state"] == state;
            seen.push(message);
            if done {
                return seen;
            }
        }
        panic!("the UI channel closed: {seen:?}")
    });
    found
        .await
        .unwrap_or_else(|_| panic!("no host_link {state} in {wait:?}"))
}

/// A host that must be refused: the link stops at once with `kind`, and nothing is uploaded.
async fn refused(alias: &str, kind: &str, reason: &str) {
    let remotes = Arc::new(Remotes::new(
        var("HIVE_CI_SSH").into(),
        var("HIVE_CI_BUILDS").into(),
        Arc::default(),
    ));
    let (channel, mut rx) = ui();
    let started = Instant::now();
    remotes.sync(settings(&[alias], 1), Some(&channel));
    let seen = reach(&mut rx, "disconnected", LONG).await;
    assert_eq!(states(&seen), ["connecting", "disconnected"], "{alias}");
    let stopped = seen.last().unwrap();
    assert_eq!(stopped["kind"], kind, "{alias}: {stopped}");
    assert_eq!(stopped["reason"], reason, "{alias}");
    remotes.end();
    report(&format!(
        "- `{alias}` refused as `{kind}` in {} ms: {reason}",
        started.elapsed().as_millis()
    ));
}

/// A terminal's output as the UI would take it: each chunk acknowledged (9.19), as xterm.js does
/// once it parsed it, and kept with the time it came.
struct Screen {
    id: u32,
    bytes: mpsc::UnboundedReceiver<Vec<u8>>,
    text: Vec<u8>,
}

impl Screen {
    fn open(hive: &Hive, host: &str) -> (Self, u32) {
        let (channel, bytes) = output();
        let link = hive.on(Some(host)).unwrap();
        let id = link
            .open_terminal("/tmp".into(), 120, 40, None, channel)
            .unwrap();
        let screen = Self {
            id,
            bytes,
            text: Vec::new(),
        };
        (screen, id)
    }

    /// The next output, acknowledged, within `wait`.
    async fn next(&mut self, hive: &Hive, wait: Duration) -> Option<Vec<u8>> {
        let chunk = tokio::time::timeout(wait, self.bytes.recv()).await.ok()??;
        hive.ack_terminal(self.id, u32::try_from(chunk.len()).unwrap())
            .unwrap();
        self.text.extend_from_slice(&chunk);
        Some(chunk)
    }

    /// Reads until the output so far holds `needle`.
    async fn until(&mut self, hive: &Hive, needle: &str, wait: Duration) {
        let deadline = Instant::now() + wait;
        while !String::from_utf8_lossy(&self.text).contains(needle) {
            let left = deadline.saturating_duration_since(Instant::now());
            let text = String::from_utf8_lossy(&self.text).into_owned();
            self.next(hive, left)
                .await
                .unwrap_or_else(|| panic!("no {needle:?} in {wait:?}: {text:?}"));
        }
    }

    /// Reads until nothing comes for `quiet`.
    async fn settle(&mut self, hive: &Hive, quiet: Duration) {
        while self.next(hive, quiet).await.is_some() {}
    }

    /// Types `keys` keys, each after the previous one's echo, alternately `x` and a backspace
    /// (as the spike, §7.1): the time from each key to its first output, in ms.
    async fn echoes(&mut self, hive: &Hive, keys: usize) -> Vec<f64> {
        let mut times = Vec::with_capacity(keys);
        for key in 0..keys {
            let typed = if key % 2 == 0 { "x" } else { "\x7f" };
            let sent = Instant::now();
            hive.write_terminal(self.id, typed).unwrap();
            self.next(hive, ECHO_WAIT)
                .await
                .unwrap_or_else(|| panic!("key {key} had no echo in {ECHO_WAIT:?}"));
            times.push(sent.elapsed().as_secs_f64() * 1000.0);
            // What else that key printed, then a pause, so the next key's echo is its own.
            self.settle(hive, Duration::from_millis(20)).await;
        }
        times
    }
}

/// p50, p95 and max of `times`, in ms.
fn spread(times: &[f64]) -> (f64, f64, f64) {
    let mut sorted = times.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |p: f64| sorted[((sorted.len() as f64 * p).ceil() as usize).saturating_sub(1)];
    (at(0.50), at(0.95), sorted[sorted.len() - 1])
}

#[tokio::test(flavor = "multi_thread")]
async fn a_real_remote_over_a_slow_lossy_link() {
    let rtt: f64 = var("HIVE_CI_RTT_MS").parse().unwrap();
    report(&format!(
        "### A real remote (26.20)\n\nHive {VERSION}; netem adds {rtt} ms and 2 % loss on the host's side.\n"
    ));

    // Hosts that are refused, before anything is uploaded (Q5, Q6).
    let unknown = "hive-unknown's host key is not known (or has changed). Run `ssh hive-unknown` once in a terminal to check and accept its key.";
    refused("hive-unknown", "host_key", unknown).await;
    let changed = "hive-changed's host key is not known (or has changed). Run `ssh hive-changed` once in a terminal to check and accept its key.";
    refused("hive-changed", "host_key", changed).await;
    let darwin = "hive-darwin runs macOS; Hive connects only to Linux hosts.";
    refused("hive-darwin", "platform", darwin).await;
    let uploaded = ssh(
        "hive-darwin",
        &["test -e .hive && echo uploaded || echo none"],
    );
    assert_eq!(uploaded, "none\n");

    // The install over ssh, then the link: the app's own routing (`Hive::on`, its terminal
    // commands) over 26.17's `Remotes`.
    let hive = Hive::new("/nonexistent/hive-ci".into(), vec![])
        .with_remote(var("HIVE_CI_SSH").into(), var("HIVE_CI_BUILDS").into());
    let hive = Arc::new(hive);
    let remotes = hive.remotes().unwrap();
    let (channel, mut rx) = ui();
    let started = Instant::now();
    remotes.sync(settings(&["hive-ci"], 60), Some(&channel));
    let seen = reach(&mut rx, "connected", LONG).await;
    assert_eq!(
        states(&seen),
        ["connecting", "installing", "connecting", "connected"]
    );
    let linked = started.elapsed();
    let build =
        std::fs::read(std::path::Path::new(&var("HIVE_CI_BUILDS")).join("hive-linux-x86_64"))
            .unwrap();
    let sum: String = Sha256::digest(&build)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let installed = ssh("hive-ci", &["sha256sum", ".hive/bin/hive"]);
    assert_eq!(installed.split_whitespace().next(), Some(sum.as_str()));
    report(&format!(
        "- Install over ssh ({} bytes) and link: {} ms from the first `ssh` to `connected`.",
        build.len(),
        linked.as_millis()
    ));

    // A terminal's echo (question 2): p95 under the round trip plus 50 ms.
    let (mut shell, id) = Screen::open(&hive, "hive-ci");
    shell.until(&hive, "$ ", LONG).await;
    shell.settle(&hive, Duration::from_millis(500)).await;
    let (p50, p95, max) = spread(&shell.echoes(&hive, KEYS).await);
    report(&format!(
        "- Echo, {KEYS} keys: p50 {p50:.1} ms, p95 {p95:.1} ms, max {max:.1} ms (limit: p95 under {} ms).",
        rtt + 50.0
    ));
    assert!(p95 < rtt + 50.0, "echo p95 {p95:.1} ms");

    // A second terminal floods (`yes`) while the first echoes: reported (spike §7.2, Q6 there).
    let (mut flood, flood_id) = Screen::open(&hive, "hive-ci");
    flood.until(&hive, "$ ", LONG).await;
    flood.settle(&hive, Duration::from_millis(500)).await;
    let flooded = Arc::new(AtomicU64::new(0));
    let pump = {
        let hive = Arc::clone(&hive);
        let count = Arc::clone(&flooded);
        let mut bytes = std::mem::replace(&mut flood.bytes, mpsc::unbounded_channel().1);
        let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    chunk = bytes.recv() => {
                        let Some(chunk) = chunk else { break };
                        let len = u32::try_from(chunk.len()).unwrap();
                        count.fetch_add(u64::from(len), SeqCst);
                        // Gone once the terminal is closed.
                        let _ = hive.ack_terminal(flood_id, len);
                    }
                    _ = &mut stopped => break,
                }
            }
        });
        (stop, task)
    };
    hive.write_terminal(flood_id, "yes\r").unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let (before, since) = (flooded.load(SeqCst), Instant::now());
    let (p50, p95, max) = spread(&shell.echoes(&hive, FLOOD_KEYS).await);
    let rate = (flooded.load(SeqCst) - before) as f64 / since.elapsed().as_secs_f64();
    report(&format!(
        "- Echo while another terminal floods, {FLOOD_KEYS} keys: p50 {p50:.1} ms, p95 {p95:.1} ms, max {max:.1} ms; the flood meanwhile {:.2} MiB/s.",
        rate / 1024.0 / 1024.0
    ));
    hive.close_terminal(flood_id).unwrap();
    let (stop, task) = pump;
    let _ = stop.send(());
    task.await.unwrap();
    shell.settle(&hive, Duration::from_secs(1)).await;

    // The link drops (its sshd session killed, from the terminal itself) and comes back: what
    // the terminal wrote meanwhile is replayed, once.
    let before = shell.text.len();
    // The echo of the line says `done-$((6*7))`; only its run says `done-42`, half a second
    // after the drop, while the ladder waits its first second.
    let line = "pkill -KILL -f '^sshd(-session)?: hive@notty'; sleep 0.5; echo done-$((6*7))\r";
    hive.write_terminal(id, line).unwrap();
    let dropped = Instant::now();
    let seen = reach(&mut rx, "connected", LONG).await;
    let back = dropped.elapsed();
    let states = states(&seen);
    assert_eq!(
        states.first().map(String::as_str),
        Some("reconnecting"),
        "{seen:?}"
    );
    assert!(!states.contains(&"installing".to_owned()), "{seen:?}");
    let reattached = next_of(&mut rx, "terminal_reattached").await;
    assert_eq!(reattached["channel"], id);
    shell.until(&hive, "done-42", LONG).await;
    let replayed = dropped.elapsed();
    shell.settle(&hive, Duration::from_secs(1)).await;
    let after = String::from_utf8_lossy(&shell.text[before..]).into_owned();
    assert_eq!(after.matches("done-42").count(), 1, "{after:?}");
    // The terminal is live again.
    shell.echoes(&hive, 4).await;
    report(&format!(
        "- Dropped link (its sshd session killed): `connected` again {} ms after the command, the output written meanwhile replayed after {} ms.",
        back.as_millis(),
        replayed.as_millis()
    ));

    hive.close_terminal(id).unwrap();
    remotes.end();
}
