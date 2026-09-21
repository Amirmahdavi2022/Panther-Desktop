// Engine supervision, kept free of any Tauri types so it can be compiled and
// tested on its own. Every engine is an ordinary executable that ends up
// publishing a local SOCKS5 proxy on 127.0.0.1 — no TUN driver, no admin rights.
//
// The one thing worth understanding before changing anything here: an engine is
// not always a single process. The Global engine cannot reach its own
// bootstrap host or its own servers from the networks this app exists for, so
// it is dialled *through* the Aether tunnel rather than straight at the
// network. That is why a "plan" is a list of steps and not one command.

use std::fs;
use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Aether's own documented default, so the address users see matches its docs.
pub const AETHER_PORT: u16 = 1819;
/// The Global engine listens one port up, because both are alive at once.
pub const GLOBAL_PORT: u16 = 1820;
/// Prowl's port, kept distinct for the same reason.
pub const PROWL_PORT: u16 = 1821;

// Taken from the Android build's Global engine so both platforms speak to the
// same network with the same identity. Attribution lives in NOTICE.md.
const PROPAGATION_CHANNEL_ID: &str = "FFFFFFFFFFFFFFFF";
const SPONSOR_ID: &str = "FFFFFFFFFFFFFFFF";
const SERVER_LIST_URL: &str =
    "https://s3.amazonaws.com//psiphon/web/mjr4-p23r-puwl/server_list_compressed";
const SERVER_LIST_KEY: &str = "MIICIDANBgkqhkiG9w0BAQEFAAOCAg0AMIICCAKCAgEAt7Ls+/39r+T6zNW7GiVpJfzq/xvL9SBH5rIFnk0RXYEYavax3WS6HOD35eTAqn8AniOwiH+DOkvgSKF2caqk/y1dfq47Pdymtwzp9ikpB1C5OfAysXzBiwVJlCdajBKvBZDerV1cMvRzCKvKwRmvDmHgphQQ7WfXIGbRbmmk6opMBh3roE42KcotLFtqp0RRwLtcBRNtCdsrVsjiI1Lqz/lH+T61sGjSjQ3CHMuZYSQJZo/KrvzgQXpkaCTdbObxHqb6/+i1qaVOfEsvjoiyzTxJADvSytVtcTjijhPEV6XskJVHE1Zgl+7rATr/pDQkw6DPCNBS1+Y6fy7GstZALQXwEDN/qhQI9kWkHijT8ns+i1vGg00Mk/6J75arLhqcodWsdeG/M/moWgqQAnlZAGVtJI1OgeF5fsPpXu4kctOfuZlGjVZXQNW34aOzm8r8S0eVZitPlbhcPiR4gT/aSMz/wd8lZlzZYsje/Jr8u/YtlwjjreZrGRmG8KMOzukV3lLmMppXFMvl4bxv6YFEmIuTsOhbLTwFgh7KYNjodLj/LsqRVfwz31PgWQFTEPICV7GCvgVlPRxnofqKSjgTWI4mxDhBpVcATvaoBl1L/6WLbFvBsoAUBItWwctO2xalKxF5szhGm8lccoc5MZr8kfE0uxMgsxz4er68iCID+rsCAQM=";

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
            Engine::Global => "global",
            Engine::Prowl => "prowl",
        }
    }

    /// The processes to bring up, in order, and the port the finished chain
    /// serves on.
    pub fn plan(self) -> Plan {
        match self {
            Engine::Aether => Plan {
                steps: vec![Step { engine: Engine::Aether, port: AETHER_PORT, upstream: None }],
                socks: AETHER_PORT,
            },
            // Dialled through Aether, never straight at the network: on its own
            // it cannot fetch its server list or reach its servers.
            Engine::Global => Plan {
                steps: vec![
                    Step { engine: Engine::Aether, port: AETHER_PORT, upstream: None },
                    Step { engine: Engine::Global, port: GLOBAL_PORT, upstream: Some(AETHER_PORT) },
                ],
                socks: GLOBAL_PORT,
            },
            Engine::Prowl => Plan {
                steps: vec![Step { engine: Engine::Prowl, port: PROWL_PORT, upstream: None }],
                socks: PROWL_PORT,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub engine: Engine,
    pub port: u16,
    pub upstream: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub socks: u16,
}

pub fn binary_path(resources: &Path, engine: Engine) -> PathBuf {
    let stem = engine.binary_stem();
    if cfg!(windows) {
        resources.join(format!("{stem}.exe"))
    } else {
        resources.join(stem)
    }
}

/// The Global engine takes its whole setup as a JSON file, so the file has to
/// be written before it starts. `LocalSocksProxyPort` is fixed rather than 0
/// because the desktop app has no callback channel to learn a random one.
pub fn write_global_config(
    data: &Path,
    port: u16,
    upstream: Option<u16>,
    region: &str,
) -> io::Result<PathBuf> {
    let store = data.join("global-core");
    fs::create_dir_all(&store)?;

    let mut fields = vec![
        format!("\"PropagationChannelId\":\"{PROPAGATION_CHANNEL_ID}\""),
        format!("\"SponsorId\":\"{SPONSOR_ID}\""),
        format!("\"RemoteServerListUrl\":\"{SERVER_LIST_URL}\""),
        format!("\"RemoteServerListSignaturePublicKey\":\"{SERVER_LIST_KEY}\""),
        "\"RemoteServerListDownloadFilename\":\"remote_server_list\"".to_string(),
        format!("\"DataRootDirectory\":{}", json_string(&store.display().to_string())),
        format!("\"MigrateDataStoreDirectory\":{}", json_string(&store.display().to_string())),
        "\"ClientPlatform\":\"Windows_Panther\"".to_string(),
        format!("\"LocalSocksProxyPort\":{port}"),
        "\"DisableLocalHTTPProxy\":true".to_string(),
        "\"EmitDiagnosticNotices\":true".to_string(),
    ];
    if let Some(up) = upstream {
        fields.push(format!("\"UpstreamProxyURL\":\"socks5://127.0.0.1:{up}\""));
    }
    // Empty means "let the engine choose". Asking for a country it is not
    // offering right now is not an error: it falls back to choosing itself.
    let region = normalise_region(region);
    if !region.is_empty() {
        fields.push(format!("\"EgressRegion\":\"{region}\""));
    }

    let path = store.join("config.json");
    fs::write(&path, format!("{{{}}}", fields.join(",")))?;
    Ok(path)
}

/// Two upper-case letters, or empty for anything that is not a country code.
/// Everything else is dropped rather than passed on, because a malformed value
/// would be written straight into the engine's config.
pub fn normalise_region(code: &str) -> String {
    let t = code.trim();
    if t.len() == 2 && t.chars().all(|c| c.is_ascii_alphabetic()) {
        t.to_ascii_uppercase()
    } else {
        String::new()
    }
}

/// The exit countries offered before the engine has listed its own. Taken from
/// the Android build so both platforms show the same starting set.
pub const REGIONS: &[&str] = &[
    "AT", "BE", "BG", "CA", "CH", "CZ", "DE", "DK", "EE", "ES", "FI", "FR", "GB", "HU", "IE",
    "IN", "IT", "JP", "LV", "NL", "NO", "PL", "RO", "RS", "SE", "SG", "SK", "UA", "US",
];

/// Escapes a value into a JSON string literal. Windows paths carry backslashes,
/// which would otherwise produce a file the engine refuses to parse.
pub fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Arguments for one step. The Global engine's config path is produced by
/// `write_global_config` and passed in.
pub fn args_for(step: Step, config: Option<&Path>) -> Vec<String> {
    match step.engine {
        // Only the address goes on the command line. Everything else about how
        // Aether connects is set through `env_for`, the same way the Android
        // build does it.
        Engine::Aether => vec![
            "--bind".into(),
            format!("127.0.0.1:{}", step.port),
        ],
        Engine::Global => vec![
            "-config".into(),
            config.map(|p| p.display().to_string()).unwrap_or_default(),
        ],
        Engine::Prowl => vec![
            "run".into(),
            "-c".into(),
            config.map(|p| p.display().to_string()).unwrap_or_default(),
        ],
    }
}

/// Environment for one step. For Aether these are the Android build's
/// defaults, which are what actually connects on the networks this app is for:
/// the gool (warp-in-warp) protocol, turbo scan, IPv4, the firewall noise
/// profile. The desktop build first shipped with `--masque` on an older core
/// and never connected, so the two platforms are kept identical on purpose.
pub fn env_for(step: Step, data: &Path) -> Vec<(String, String)> {
    match step.engine {
        Engine::Aether => vec![
            ("AETHER_PROTOCOL".into(), "gool".into()),
            ("AETHER_SCAN".into(), "turbo".into()),
            ("AETHER_IP".into(), "v4".into()),
            ("AETHER_NOIZE".into(), "firewall".into()),
            ("AETHER_LOG_LEVEL".into(), "info".into()),
            ("AETHER_QUICK_RECONNECT".into(), "1".into()),
            ("AETHER_WG_STALE_SECS".into(), "30".into()),
            ("AETHER_SOCKS".into(), format!("127.0.0.1:{}", step.port)),
            // Its identity file. Left alone it lands in the working directory.
            ("AETHER_CONFIG".into(), data.join("aether.toml").display().to_string()),
        ],
        _ => Vec::new(),
    }
}

/// How long a step may take to open its port. Matches the Android build:
/// Aether's first run has to register an account and scan for a gateway
/// before it listens, which regularly takes well over the 45 s this used to
/// allow, and Global needs longer again because it rides on Aether.
pub fn step_timeout(engine: Engine) -> Duration {
    match engine {
        Engine::Global => Duration::from_secs(150),
        _ => Duration::from_secs(120),
    }
}

/// Where a core's output goes. Rewritten on every start, so it always holds
/// the attempt that just happened.
pub fn log_path(data: &Path, engine: Engine) -> PathBuf {
    data.join("logs").join(format!("{}.log", engine.binary_stem()))
}

/// True once something is accepting connections on the local port. This is the
/// only honest signal that an engine is up: a process that started and then
/// died would otherwise look like success.
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
    children: Vec<Child>,
    current: Option<Engine>,
    socks: Option<u16>,
    region: String,
    resources: PathBuf,
    data: PathBuf,
}

impl Supervisor {
    pub fn new(resources: PathBuf, data: PathBuf) -> Self {
        Self {
            children: Vec::new(),
            current: None,
            socks: None,
            region: String::new(),
            resources,
            data,
        }
    }

    pub fn current(&self) -> Option<Engine> {
        self.current
    }

    pub fn socks_port(&self) -> Option<u16> {
        self.socks
    }

    /// The exit country currently in force, or empty for automatic.
    pub fn region(&self) -> &str {
        &self.region
    }

    pub fn is_running(&mut self) -> bool {
        !self.children.is_empty()
            && self.children.iter_mut().all(|c| matches!(c.try_wait(), Ok(None)))
    }

    /// Brings up every step of the engine's plan in order. A step that never
    /// opens its port tears the whole chain down rather than leaving half of it
    /// running, because a half-started chain is indistinguishable from a
    /// working one from the outside.
    pub fn start(&mut self, engine: Engine) -> io::Result<u16> {
        self.start_in(engine, "")
    }

    /// `region` is an ISO country code for the exit, or empty for automatic.
    /// Only the Global engine can honour it; the others ignore it.
    pub fn start_in(&mut self, engine: Engine, region: &str) -> io::Result<u16> {
        self.stop();
        let region = normalise_region(region);
        let plan = engine.plan();

        for step in &plan.steps {
            let bin = binary_path(&self.resources, step.engine);
            if !bin.exists() {
                self.stop();
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{} core was not found at {}", step.engine.label(), bin.display()),
                ));
            }

            let config = match step.engine {
                Engine::Global => {
                    Some(write_global_config(&self.data, step.port, step.upstream, &region)?)
                }
                _ => None,
            };

            // The core's own output is the only way to see why a connection
            // failed on someone else's machine, so it goes to a file.
            let log = log_path(&self.data, step.engine);
            let (out, err) = match log.parent().map(fs::create_dir_all) {
                Some(Ok(())) => match fs::File::create(&log) {
                    Ok(f) => match f.try_clone() {
                        Ok(g) => (Stdio::from(f), Stdio::from(g)),
                        Err(_) => (Stdio::from(f), Stdio::null()),
                    },
                    Err(_) => (Stdio::null(), Stdio::null()),
                },
                _ => (Stdio::null(), Stdio::null()),
            };

            let mut cmd = Command::new(&bin);
            cmd.args(args_for(*step, config.as_deref()))
                .envs(env_for(*step, &self.data))
                .stdin(Stdio::null())
                .stdout(out)
                .stderr(err);
            // A writable place of its own, rather than wherever the app was
            // launched from.
            if fs::create_dir_all(&self.data).is_ok() {
                cmd.current_dir(&self.data);
            }

            // Keep a console window from flashing up on Windows.
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }

            match cmd.spawn() {
                Ok(child) => self.children.push(child),
                Err(e) => {
                    self.stop();
                    return Err(e);
                }
            }

            // The first hop has to find a route before the next one can use it,
            // so each step gets its own generous window.
            // Watch the process as well as the port: a core that has already
            // exited is never going to open it, and waiting out the full
            // timeout for a dead process just looks like a hang.
            let deadline = Instant::now() + step_timeout(step.engine);
            let mut up = false;
            while Instant::now() < deadline {
                if port_is_open(step.port) {
                    up = true;
                    break;
                }
                let exited = self
                    .children
                    .last_mut()
                    .map(|c| matches!(c.try_wait(), Ok(Some(_))))
                    .unwrap_or(true);
                if exited {
                    self.stop();
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionRefused,
                        format!(
                            "{} exited before it could connect; see {}",
                            step.engine.label(),
                            log.display()
                        ),
                    ));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            if !up {
                self.stop();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "{} started but never accepted connections; see {}",
                        step.engine.label(),
                        log.display()
                    ),
                ));
            }
        }

        self.current = Some(engine);
        self.socks = Some(plan.socks);
        self.region = region;
        Ok(plan.socks)
    }

    pub fn stop(&mut self) {
        // Last started, first stopped: the outer hop should not lose its route
        // out from under it while it is still shutting down.
        while let Some(mut c) = self.children.pop() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.current = None;
        self.socks = None;
        self.region.clear();
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.stop();
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
            assert_eq!(name, "prowl.exe");
        } else {
            assert_eq!(name, "prowl");
        }
    }

    #[test]
    fn aether_runs_alone_on_its_documented_port() {
        let plan = Engine::Aether.plan();
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.socks, 1819);
        assert_eq!(plan.steps[0].upstream, None);
    }

    /// The whole reason the Global engine works at all: it is carried inside
    /// Aether. If this ever becomes a single step it will connect on a free
    /// network and fail on the networks that matter.
    #[test]
    fn global_is_chained_through_aether() {
        let plan = Engine::Global.plan();
        assert_eq!(plan.steps.len(), 2, "Global must be dialled through Aether");
        assert_eq!(plan.steps[0].engine, Engine::Aether);
        assert_eq!(plan.steps[1].engine, Engine::Global);
        assert_eq!(plan.steps[1].upstream, Some(AETHER_PORT));
        assert_eq!(plan.socks, GLOBAL_PORT, "the chain serves on the outer hop's port");
    }

    #[test]
    fn chained_steps_never_share_a_port() {
        let plan = Engine::Global.plan();
        assert_ne!(plan.steps[0].port, plan.steps[1].port);
    }

    #[test]
    fn aether_is_told_which_port_to_bind() {
        let step = Step { engine: Engine::Aether, port: 1819, upstream: None };
        let args = args_for(step, None).join(" ");
        assert!(args.contains("127.0.0.1:1819"), "got: {args}");
    }

    #[test]
    fn aether_gets_the_android_settings() {
        let step = Step { engine: Engine::Aether, port: 1819, upstream: None };
        let env = env_for(step, Path::new("data"));
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(get("AETHER_PROTOCOL").as_deref(), Some("gool"));
        assert_eq!(get("AETHER_SCAN").as_deref(), Some("turbo"));
        assert_eq!(get("AETHER_IP").as_deref(), Some("v4"));
        assert_eq!(get("AETHER_NOIZE").as_deref(), Some("firewall"));
        assert_eq!(get("AETHER_SOCKS").as_deref(), Some("127.0.0.1:1819"));
        assert!(get("AETHER_CONFIG").unwrap().ends_with("aether.toml"));
        // masque never connected on the desktop build; keep it off the command line.
        assert!(!args_for(step, None).iter().any(|a| a == "--masque"));
    }

    #[test]
    fn timeouts_match_the_android_build() {
        assert_eq!(step_timeout(Engine::Aether), Duration::from_secs(120));
        assert_eq!(step_timeout(Engine::Global), Duration::from_secs(150));
    }

    #[test]
    fn windows_paths_survive_json_escaping() {
        let escaped = json_string(r"C:\Users\Amir\AppData\Panther");
        assert_eq!(escaped, r#""C:\\Users\\Amir\\AppData\\Panther""#);
        // Every backslash must be doubled, or the engine refuses the file.
        let raw = r"C:\Users\Amir\AppData\Panther";
        assert_eq!(
            escaped.matches('\\').count(),
            raw.matches('\\').count() * 2,
            "each backslash must be escaped exactly once"
        );
    }

    #[test]
    fn global_config_carries_the_upstream_and_the_port() {
        let dir = std::env::temp_dir().join(format!("panther-cfg-{}", std::process::id()));
        let path = write_global_config(&dir, 1820, Some(1819), "").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"LocalSocksProxyPort\":1820"), "{text}");
        assert!(text.contains("socks5://127.0.0.1:1819"), "{text}");
        assert!(text.contains("PropagationChannelId"), "{text}");
        // It has to be parseable JSON, not merely look like it.
        assert!(text.starts_with('{') && text.ends_with('}'));
        assert_eq!(text.matches('{').count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_standalone_global_config_has_no_upstream() {
        let dir = std::env::temp_dir().join(format!("panther-cfg2-{}", std::process::id()));
        let path = write_global_config(&dir, 1820, None, "").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("UpstreamProxyURL"), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_chosen_country_reaches_the_config() {
        let dir = std::env::temp_dir().join(format!("panther-cfg3-{}", std::process::id()));
        let path = write_global_config(&dir, 1820, Some(1819), "nl").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"EgressRegion\":\"NL\""), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn automatic_means_no_region_at_all() {
        let dir = std::env::temp_dir().join(format!("panther-cfg4-{}", std::process::id()));
        for asked in ["", "  ", "auto", "NLD", "1"] {
            let path = write_global_config(&dir, 1820, None, asked).unwrap();
            let text = fs::read_to_string(&path).unwrap();
            assert!(!text.contains("EgressRegion"), "{asked:?} produced: {text}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn region_codes_are_normalised_or_dropped() {
        assert_eq!(normalise_region("nl"), "NL");
        assert_eq!(normalise_region(" de "), "DE");
        assert_eq!(normalise_region("USA"), "");
        assert_eq!(normalise_region("u1"), "");
        assert_eq!(normalise_region(""), "");
    }

    #[test]
    fn the_offered_regions_are_well_formed() {
        assert!(REGIONS.len() >= 20);
        for r in REGIONS {
            assert_eq!(normalise_region(r), **r, "{r} is not a clean country code");
        }
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
        let mut s = Supervisor::new(
            PathBuf::from("/definitely/not/here"),
            std::env::temp_dir(),
        );
        let err = s.start(Engine::Aether).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(!s.is_running());
        assert_eq!(s.socks_port(), None);
    }
}
