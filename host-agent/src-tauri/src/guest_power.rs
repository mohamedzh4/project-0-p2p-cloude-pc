//! Stopping guest VMs: cleanly when possible, and also the ones a previous
//! run of the agent left behind.
//!
//! - **Clean shutdown (issue 21).** Every guest gets a QEMU guest agent
//!   channel on a loopback TCP port (the signed image ships
//!   `qemu-guest-agent`). At session end the Host Agent asks the guest to
//!   power off, so systemd stops `dcl-guest-join`, whose stop step waits for
//!   rclone to finish uploading `~/Persistent`. Only then, or after
//!   [`SHUTDOWN_GRACE`], is QEMU killed.
//! - **Leftovers (issue 32).** Every guest is started with `-name`
//!   [`GUEST_NAME`]. When the agent starts, any QEMU process carrying that
//!   name belongs to an earlier run that closed or crashed without a
//!   teardown: it is shut down the same way, then killed.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Marks QEMU processes started by this agent (`-name`).
pub const GUEST_NAME: &str = "dcl-host-agent-guest";
/// How long a guest may take to upload its files and power off.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(90);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const PING_TIMEOUT: Duration = Duration::from_secs(5);
const EXIT_POLL: Duration = Duration::from_millis(500);
const QGA_CHARDEV: &str = "dclqga";

/// Guest agent ports of guests started by this process.
static PORTS: Mutex<Vec<u16>> = Mutex::new(Vec::new());

/// A loopback port nothing listens on right now, for the guest agent channel.
pub fn free_local_port() -> Option<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    listener.local_addr().ok().map(|addr| addr.port())
}

/// `-name` plus a guest agent channel on `127.0.0.1:port`, when there is one.
pub fn guest_argv(qga_port: Option<u16>) -> Vec<String> {
    let mut args = vec!["-name".to_string(), GUEST_NAME.to_string()];
    if let Some(port) = qga_port {
        args.extend([
            "-chardev".to_string(),
            format!("socket,id={QGA_CHARDEV},host=127.0.0.1,port={port},server=on,wait=off"),
            "-device".to_string(),
            "virtio-serial-pci,id=dclvser".to_string(),
            "-device".to_string(),
            format!(
                "virtserialport,bus=dclvser.0,chardev={QGA_CHARDEV},name=org.qemu.guest_agent.0"
            ),
        ]);
    }
    args
}

/// Picks a port and returns it with the argv to add. Call [`remember`] once
/// QEMU started.
pub fn prepare() -> (Option<u16>, Vec<String>) {
    let port = free_local_port();
    (port, guest_argv(port))
}

pub fn remember(port: Option<u16>) {
    if let Some(port) = port {
        if let Ok(mut ports) = PORTS.lock() {
            ports.push(port);
        }
    }
}

/// The guest agent port in a QEMU command line built by [`guest_argv`].
pub fn qga_port_from_cmdline(cmd: &[String]) -> Option<u16> {
    let prefix = format!("socket,id={QGA_CHARDEV},");
    cmd.iter()
        .filter(|arg| arg.starts_with(&prefix))
        .find_map(|arg| {
            arg.split(',')
                .find_map(|part| part.strip_prefix("port="))
                .and_then(|port| port.parse().ok())
        })
}

fn connect(port: u16) -> Option<TcpStream> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).ok()
}

/// Asks the guest to power off. False when its guest agent did not answer
/// (not booted yet, or no agent), so there is no point waiting.
pub fn request_poweroff(port: u16) -> bool {
    let Some(stream) = connect(port) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(PING_TIMEOUT));
    let _ = stream.set_write_timeout(Some(PING_TIMEOUT));
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(_) => return false,
    };
    // guest-sync flushes anything a previous client left in the channel.
    let sync_id = 0x5e55_1011_u32;
    if writer
        .write_all(
            format!("{{\"execute\":\"guest-sync\",\"arguments\":{{\"id\":{sync_id}}}}}\n").as_bytes(),
        )
        .is_err()
    {
        return false;
    }
    let mut reader = BufReader::new(stream);
    let mut answered = false;
    let mut line = String::new();
    while reader.read_line(&mut line).map(|n| n > 0).unwrap_or(false) {
        if line.contains(&sync_id.to_string()) {
            answered = true;
            break;
        }
        line.clear();
    }
    if !answered {
        return false;
    }
    writer
        .write_all(b"{\"execute\":\"guest-shutdown\",\"arguments\":{\"mode\":\"powerdown\"}}\n")
        .is_ok()
}

/// True once nothing accepts connections on the port: QEMU has exited.
fn exited(port: u16) -> bool {
    connect(port).is_none()
}

/// Ask every guest on `ports` to power off and wait up to `grace` for them
/// to exit. Returns how many exited on their own.
pub fn shut_down(ports: &[u16], grace: Duration) -> usize {
    let asked: Vec<u16> = ports
        .iter()
        .copied()
        .filter(|port| request_poweroff(*port))
        .collect();
    if asked.is_empty() {
        return 0;
    }
    crate::agent_log!(
        "host-agent: asked {} guest(s) to save their files and power off",
        asked.len()
    );
    let deadline = Instant::now() + grace;
    loop {
        let done = asked.iter().filter(|port| exited(**port)).count();
        if done == asked.len() || Instant::now() >= deadline {
            return done;
        }
        std::thread::sleep(EXIT_POLL);
    }
}

/// Clean shutdown of every guest this process started. QEMU is killed by the
/// caller afterwards either way.
pub fn shut_down_own_guests() {
    let ports: Vec<u16> = PORTS
        .lock()
        .map(|mut ports| ports.drain(..).collect())
        .unwrap_or_default();
    if ports.is_empty() {
        return;
    }
    let clean = shut_down(&ports, SHUTDOWN_GRACE);
    crate::agent_log!(
        "host-agent: {clean} of {} guest(s) powered off cleanly",
        ports.len()
    );
}

/// Stop QEMU guests an earlier run of the agent left running.
pub fn stop_leftover_guests() -> usize {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
    );
    let own = std::process::id();
    let leftovers: Vec<(sysinfo::Pid, Option<u16>)> = system
        .processes()
        .iter()
        .filter(|(pid, _)| pid.as_u32() != own)
        .filter_map(|(pid, process)| {
            let cmd: Vec<String> = process
                .cmd()
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            is_leftover_guest(&cmd).then(|| (*pid, qga_port_from_cmdline(&cmd)))
        })
        .collect();
    if leftovers.is_empty() {
        return 0;
    }
    crate::agent_log!(
        "host-agent: found {} guest VM(s) left by an earlier run; stopping them",
        leftovers.len()
    );
    let ports: Vec<u16> = leftovers.iter().filter_map(|(_, port)| *port).collect();
    shut_down(&ports, SHUTDOWN_GRACE);
    system.refresh_processes(ProcessesToUpdate::All, true);
    for (pid, _) in &leftovers {
        if let Some(process) = system.process(*pid) {
            process.kill();
        }
    }
    leftovers.len()
}

/// A QEMU command line carrying this agent's guest name.
pub fn is_leftover_guest(cmd: &[String]) -> bool {
    let is_qemu = cmd.first().is_some_and(|program| {
        let lower = program.to_ascii_lowercase();
        lower.contains("qemu-system")
    });
    is_qemu
        && cmd
            .windows(2)
            .any(|pair| pair[0] == "-name" && pair[1].split(',').next() == Some(GUEST_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn guest_argv_names_the_guest_and_adds_the_agent_channel() {
        let args = guest_argv(Some(40123));
        assert_eq!(&args[..2], ["-name", GUEST_NAME]);
        assert!(args.contains(
            &"socket,id=dclqga,host=127.0.0.1,port=40123,server=on,wait=off".to_string()
        ));
        assert!(args
            .iter()
            .any(|arg| arg.ends_with("name=org.qemu.guest_agent.0")));
        assert_eq!(qga_port_from_cmdline(&args), Some(40123));
        assert_eq!(guest_argv(None), vec!["-name", GUEST_NAME]);
    }

    #[test]
    fn only_this_agents_qemu_counts_as_a_leftover() {
        let mut ours = vec!["/usr/bin/qemu-system-x86_64".to_string()];
        ours.extend(guest_argv(Some(1)));
        assert!(is_leftover_guest(&ours));
        let windows = vec![
            "C:\\dcl\\qemu\\QEMU-SYSTEM-X86_64.EXE".to_string(),
            "-name".to_string(),
            GUEST_NAME.to_string(),
        ];
        assert!(is_leftover_guest(&windows));
        let someone_elses = vec![
            "/usr/bin/qemu-system-x86_64".to_string(),
            "-name".to_string(),
            "my-vm".to_string(),
        ];
        assert!(!is_leftover_guest(&someone_elses));
        let not_qemu = vec!["bash".to_string(), "-name".to_string(), GUEST_NAME.to_string()];
        assert!(!is_leftover_guest(&not_qemu));
    }

    /// A fake guest agent: answers guest-sync, records the shutdown, then
    /// stops listening like an exiting QEMU.
    #[test]
    fn a_guest_that_answers_is_powered_off_and_waited_for() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.contains("guest-sync"));
            let id: String = line
                .split("\"id\":")
                .nth(1)
                .unwrap()
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            writer
                .write_all(format!("{{\"return\": {id}}}\n").as_bytes())
                .unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            drop(listener);
            line
        });
        let clean = shut_down(&[port], Duration::from_secs(10));
        let command = server.join().unwrap();
        assert!(command.contains("guest-shutdown"));
        assert!(command.contains("powerdown"));
        assert_eq!(clean, 1);
    }

    #[test]
    fn a_guest_without_an_agent_is_not_waited_for() {
        let port = free_local_port().unwrap();
        let started = Instant::now();
        assert_eq!(shut_down(&[port], Duration::from_secs(30)), 0);
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
