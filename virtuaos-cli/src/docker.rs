//! `virtuaos docker` — Docker management via the local runner socket.
//!
//! The runner (`auxinuxvirtual-runner`) exposes a JSONL socket at
//! `/run/auxinuxvirtual.sock`. This module speaks that protocol directly so the
//! host CLI can list, inspect, exec and prune containers without the Node CLI.

use anyhow::{bail, Context, Result};
use owo_colors::OwoColorize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{SystemTime, UNIX_EPOCH};

const SOCK_PATH: &str = "/run/auxinuxvirtual.sock";

fn require_root() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("Docker management requires root. Run this command with sudo");
    }
    Ok(())
}

/// Send one request to the runner socket and return its `result` value.
///
/// The runner answers with one or more JSON lines sharing the request `id`
/// (progress updates carry `progress`, the final line carries `ok`/`error`).
/// We read line-by-line and stop at the first final response for our id.
fn call_runner(action: &str, params: Value) -> Result<Value> {
    let mut stream = UnixStream::connect(SOCK_PATH).with_context(|| {
        format!("cannot connect to runner socket {SOCK_PATH} (is auxinuxvirtual-runner running?)")
    })?;

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let id = format!("vos-{}-{nanos}", std::process::id());

    let req = json!({ "id": id, "action": action, "params": params });
    let mut line = serde_json::to_string(&req)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()?;

    let reader = BufReader::new(stream.try_clone()?);
    for raw in reader.lines() {
        let raw = raw?;
        let v: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("id").and_then(|x| x.as_str()) != Some(id.as_str()) {
            continue;
        }
        if v.get("ok").and_then(|x| x.as_bool()) == Some(true) {
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
        let err = v.get("error").and_then(|x| x.as_str()).unwrap_or("runner error");
        bail!("{err}");
    }
    bail!("runner closed the connection without a response");
}

fn color_state(state: &str) -> String {
    let s = state.to_lowercase();
    if s.contains("running") || s.contains("up") || s.contains("active") {
        state.green().to_string()
    } else if s.contains("stopped") || s.contains("exited") || s.contains("dead") || s.contains("failed") {
        state.red().to_string()
    } else if s.contains("paused") || s.contains("restarting") {
        state.yellow().to_string()
    } else {
        state.to_string()
    }
}

fn fmt_bytes(n: f64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0} {}", units[i])
    } else {
        format!("{v:.1} {}", units[i])
    }
}

/// `vos docker ps` — list containers (name, image, ports, state).
pub fn ps() -> Result<()> {
    require_root()?;
    let result = call_runner("docker_containers", json!({}))?;
    let containers = result.as_array().cloned().unwrap_or_default();
    if containers.is_empty() {
        println!("{}", "(no containers)".dimmed());
        return Ok(());
    }
    println!(
        "{:<24} {:<32} {:<20} {}",
        "NAME".bold(),
        "IMAGE".bold(),
        "PORTS".bold(),
        "STATE".bold()
    );
    for c in containers {
        let name = c.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let image = c.get("image").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let state = c.get("state").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let ports = c
            .get("ports")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        let hp = p.get("hostPort").and_then(|x| x.as_u64());
                        let cp = p.get("containerPort").and_then(|x| x.as_u64());
                        match (hp, cp) {
                            (Some(h), Some(c)) => Some(format!("{h}:{c}")),
                            _ => None,
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        println!("{:<24} {:<32} {:<20} {}", name, image, ports, color_state(&state));
    }
    Ok(())
}

/// `vos docker logs <id>` — print container logs.
pub fn logs(id: &str, tail: u64) -> Result<()> {
    require_root()?;
    let result = call_runner("docker_logs", json!({ "id": id, "tail": tail }))?;
    let text = result.as_str().unwrap_or("");
    print!("{text}");
    Ok(())
}

/// `vos docker stats <id>` — one-shot resource usage snapshot.
pub fn stats(id: &str) -> Result<()> {
    require_root()?;
    let result = call_runner("docker_stats", json!({ "id": id }))?;
    let cpu = result.get("cpuPercent").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let mem_used = result.get("memUsedBytes").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let mem_limit = result.get("memLimitBytes").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let net_rx = result.get("netRxBytes").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let net_tx = result.get("netTxBytes").and_then(|x| x.as_f64()).unwrap_or(0.0);
    let pids = result.get("pids").and_then(|x| x.as_u64()).unwrap_or(0);
    println!("  {:<12} {:.2}%", "CPU", cpu);
    println!("  {:<12} {} / {}", "Memory", fmt_bytes(mem_used), fmt_bytes(mem_limit));
    println!("  {:<12} {} ↓ / {} ↑", "Network", fmt_bytes(net_rx), fmt_bytes(net_tx));
    println!("  {:<12} {}", "PIDs", pids);
    Ok(())
}

/// `vos docker exec <id> <cmd...>` — run a command inside a container.
pub fn exec(id: &str, command: &[String]) -> Result<()> {
    require_root()?;
    let cmd = command.join(" ");
    let result = call_runner("docker_exec", json!({ "id": id, "command": cmd }))?;
    let stdout = result.get("stdout").and_then(|x| x.as_str()).unwrap_or("");
    let stderr = result.get("stderr").and_then(|x| x.as_str()).unwrap_or("");
    if !stdout.is_empty() {
        print!("{stdout}");
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }
    Ok(())
}

/// `vos docker start|stop|restart <id>` — lifecycle control.
pub fn action(id: &str, action: &str) -> Result<()> {
    require_root()?;
    call_runner("docker_action", json!({ "id": id, "action": action }))?;
    println!("{} {id}: ok", action.green());
    Ok(())
}

/// `vos docker prune [target]` — reclaim unused Docker resources.
pub fn prune(target: &str) -> Result<()> {
    require_root()?;
    let result = call_runner("docker_prune", json!({ "target": target }))?;
    if let Some(results) = result.get("results").and_then(|x| x.as_object()) {
        for (k, v) in results {
            let text = v.as_str().unwrap_or("");
            if !text.is_empty() {
                println!("{}:\n{text}", k.bold());
            }
        }
    }
    println!("{}", "prune complete".green());
    Ok(())
}
