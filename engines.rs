// Engine supervision, kept free of any Tauri types so it can be compiled and
// tested on its own. Each engine is an ordinary executable that exposes a local
// SOCKS proxy on 127.0.0.1 — no TUN driver and no administrator rights.

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Aether's own documented default, kept so the address users see
/// matches the upstream docs.
pub const SOCKS_PORT: u16 = 1819;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Aether,
    Global,
    Prowl,
}

impl Engine {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "aether" => Some(Engine::Aether),
            "global" => Some(Engine::Global),
            "prowl" => Some(Engine::Prowl),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Engine::Aether => "Aether",
            Engine::Global => "Global",
            Engine::Prowl => "Prowl",
        }
    }

    /// File name of the bundled executable, without the .exe suffix.
    pub fn binary_stem(self) -> &'static str {
        match self {
            Engine::Aether => "aether",
            Engine::Global => "psiphon",
            Engine::Prowl => "xray",
        }
    }

    /// Arguments that make the engine listen as a SOCKS proxy on `port`.
    pub fn args(self, port: u16, resources: &Path) -> Vec<String> {
        match self {
            // Flags taken from Aether's own README: --masque is the documented
            // main transport and --bind fixes the local SOCKS5 address.
            Engine::Aether => vec![
                "--masque".into(),
                "--bind".into(),
                format!("127.0.0.1:{port}"),
            ],
            Engine::Global => vec![
                "-config".into(),
                resources.join("psiphon.config.json").display().to_string(),
            ],
            Engine::Prowl => vec![
                "run".into(),
                "-c".into(),
                resources.join("xray.config.json").display().to_string(),
            ],
        }
    }
}

pub fn binary_path(resources: &Path, engine: Engine) -> PathBuf {
    let stem = engine.binary_stem();
    if cfg!(windows) {
        resources.join(format!("{stem}.exe"))
    } else {
        resources.join(stem)
    }
}

/// True once something is accepting connections on the local port. This is the
/// only honest signal that an engine is actually up: a process that started and
/// then died would otherwise look like a successful connection.
pub fn port_is_open(port: u16) -> bool {
    let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    TcpStream::connect_timeout(&addr.into(), Duration::from_millis(250)).is_ok()
}

pub fn wait_for_port(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if port_is_open(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

pub struct Supervisor {
    child: Option<Child>,
    current: Option<Engine>,
    resources: PathBuf,
}

impl Supervisor {
    pub fn new(resources: PathBuf) -> Self {
        Self { child: None, current: None, resources }
    }

    pub fn current(&self) -> Option<Engine> {
        self.current
    }

    pub fn is_running(&mut self) -> bool {
        match self.child.as_mut() {
            // try_wait returns Ok(None) while the process is still alive
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }

    pub fn start(&mut self, engine: Engine, port: u16) -> io::Result<()> {
        self.stop();

        let bin = binary_path(&self.resources, engine);
        if !bin.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} core was not found at {}", engine.label(), bin.display()),
            ));
        }

        let mut cmd = Command::new(&bin);
        cmd.args(engine.args(port, &self.resources))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // Keep the console window from flashing up on Windows.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = cmd.spawn()?;
        self.child = Some(child);
        self.current = Some(engine);

        if !wait_for_port(port, Duration::from_secs(20)) {
            self.stop();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{} started but never accepted connections", engine.label()),
            ));
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.current = None;
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn human_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    let n = n as f64;
    if n < KB {
        format!("{n:.0} B")
    } else if n < KB * KB {
        format!("{:.1} KB", n / KB)
    } else if n < KB * KB * KB {
        format!("{:.1} MB", n / (KB * KB))
    } else {
        format!("{:.2} GB", n / (KB * KB * KB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn parses_known_engines_and_rejects_others() {
        assert_eq!(Engine::parse("aether"), Some(Engine::Aether));
        assert_eq!(Engine::parse("GLOBAL"), Some(Engine::Global));
        assert_eq!(Engine::parse("Prowl"), Some(Engine::Prowl));
        assert_eq!(Engine::parse("lantern"), None);
        assert_eq!(Engine::parse(""), None);
    }

    #[test]
    fn binary_path_matches_the_host_platform() {
        let p = binary_path(Path::new("/res"), Engine::Prowl);
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if cfg!(windows) {
            assert_eq!(name, "xray.exe");
        } else {
            assert_eq!(name, "xray");
        }
    }

    #[test]
    fn every_engine_gets_arguments() {
        for e in [Engine::Aether, Engine::Global, Engine::Prowl] {
            assert!(!e.args(SOCKS_PORT, Path::new("/res")).is_empty(), "{:?}", e);
        }
    }

    #[test]
    fn aether_is_told_which_port_to_bind() {
        let args = Engine::Aether.args(1819, Path::new("/res")).join(" ");
        assert!(args.contains("127.0.0.1:1819"), "got: {args}");
        assert!(args.contains("--masque"), "got: {args}");
    }

    #[test]
    fn default_port_matches_aether_docs() {
        assert_eq!(SOCKS_PORT, 1819);
    }

    #[test]
    fn port_probe_distinguishes_open_from_closed() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(port_is_open(port), "a bound port must read as open");
        drop(listener);
        assert!(!port_is_open(port), "a released port must read as closed");
    }

    #[test]
    fn starting_a_missing_binary_is_an_error_not_a_panic() {
        let mut s = Supervisor::new(PathBuf::from("/definitely/not/here"));
        let err = s.start(Engine::Aether, SOCKS_PORT).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(!s.is_running());
    }

    #[test]
    fn byte_formatting_reads_naturally() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }
}
