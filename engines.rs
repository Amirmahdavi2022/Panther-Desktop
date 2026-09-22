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
/// sing-box's local control API. Nothing talks to it; it opening is simply the
/// signal that sing-box got through its start-up, TUN included.
pub const TUN_API_PORT: u16 = 1830;
/// The processes whose own traffic must never be sent back into the tunnel,
/// or every packet they send would loop through themselves.
pub const TUN_BYPASS: &[&str] = &["aether.exe", "global.exe", "prowl.exe", "sing-box.exe"];

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

pub fn tun_binary(resources: &Path) -> PathBuf {
    if cfg!(windows) {
        resources.join("sing-box.exe")
    } else {
        resources.join("sing-box")
    }
}

/// The whole-system tunnel: sing-box owns a TUN adapter, takes every app's
/// traffic and hands it to the engine's local SOCKS port. This is what makes
/// the desktop build a real VPN instead of a proxy people have to configure.
///
/// - The cores themselves go out directly (`TUN_BYPASS`), or their own traffic
///   would be fed back into them.
/// - DNS is answered inside the tunnel, over TCP through the engine, so the ISP
///   never sees the lookups. IPv4 only, because the engines are IPv4 only;
///   IPv6 is captured and refused so apps fall back to IPv4 at once instead of
///   leaking around the tunnel or hanging.
/// - Validated with `sing-box check` against the pinned version (1.13.21).
pub fn tun_config(socks_port: u16) -> String {
    let bypass = TUN_BYPASS
        .iter()
        .map(|p| json_string(p))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{
  "log": {{"level": "warn", "timestamp": true}},
  "dns": {{
    "servers": [{{"type": "tcp", "tag": "remote", "server": "1.1.1.1", "detour": "proxy"}}],
    "strategy": "ipv4_only",
    "final": "remote"
  }},
  "inbounds": [{{
    "type": "tun",
    "tag": "tun-in",
    "interface_name": "Panther",
    "address": ["172.19.0.1/30", "fdfe:dcba:9876::1/126"],
    "mtu": 9000,
    "auto_route": true,
    "strict_route": true,
    "stack": "mixed"
  }}],
  "outbounds": [
    {{"type": "socks", "tag": "proxy", "server": "127.0.0.1", "server_port": {socks_port}, "version": "5"}},
    {{"type": "direct", "tag": "direct"}}
  ],
  "route": {{
    "auto_detect_interface": true,
    "find_process": true,
    "rules": [
      {{"process_name": [{bypass}], "outbound": "direct"}},
      {{"action": "sniff"}},
      {{"protocol": "dns", "action": "hijack-dns"}},
      {{"ip_is_private": true, "outbound": "direct"}},
      {{"ip_version": 6, "action": "reject"}}
    ],
    "final": "proxy"
  }},
  "experimental": {{
    "clash_api": {{"external_controller": "127.0.0.1:{TUN_API_PORT}"}}
  }}
}}
"#
    )
}

/// Asks, through the engine itself, which address and country the internet
/// sees. The app shows this instead of trusting what was requested, so a
/// country that was asked for but not granted is visible rather than hidden.
///
/// Plain HTTP on purpose: the request already travels inside the tunnel, and
/// it keeps this free of a TLS stack. ip-api.com's free endpoint is HTTP only.
pub fn exit_check(socks_port: u16) -> io::Result<(String, String)> {
    exit_check_via(socks_port, "ip-api.com", 80, "/json/?fields=query,countryCode")
}

pub fn exit_check_via(
    socks_port: u16,
    host: &str,
    port: u16,
    path: &str,
) -> io::Result<(String, String)> {
    use std::io::{Read, Write};
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());

    let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, socks_port);
    let mut s = TcpStream::connect_timeout(&addr.into(), Duration::from_secs(5))?;
    s.set_read_timeout(Some(Duration::from_secs(15)))?;
    s.set_write_timeout(Some(Duration::from_secs(15)))?;

    // SOCKS5, no authentication, CONNECT by host name so the lookup happens at
    // the far end of the tunnel.
    s.write_all(&[5, 1, 0])?;
    let mut reply = [0u8; 2];
    s.read_exact(&mut reply)?;
    if reply != [5, 0] {
        return Err(bad("socks: no acceptable auth method"));
    }
    if host.len() > 255 {
        return Err(bad("socks: host name too long"));
    }
    let mut req = vec![5, 1, 0, 3, host.len() as u8];
    req.extend_from_slice(host.as_bytes());
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req)?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head)?;
    if head[1] != 0 {
        return Err(bad("socks: connect refused"));
    }
    let skip = match head[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut n = [0u8; 1];
            s.read_exact(&mut n)?;
            n[0] as usize + 2
        }
        _ => return Err(bad("socks: bad reply")),
    };
    let mut rest = vec![0u8; skip];
    s.read_exact(&mut rest)?;

    // One write for the whole request, so it leaves as one piece.
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: Panther\r\nConnection: close\r\n\r\n"
    );
    s.write_all(request.as_bytes())?;
    // Stop at the end of the JSON object rather than waiting for the far end
    // to close: some proxies keep the socket open well after the reply.
    let mut body = String::new();
    let mut buf = [0u8; 2048];
    loop {
        let n = match s.read(&mut buf) {
            Ok(n) => n,
            Err(e) if body.is_empty() => return Err(e),
            Err(_) => break,
        };
        if n == 0 {
            break;
        }
        body.push_str(&String::from_utf8_lossy(&buf[..n]));
        if let Some(h) = body.find("\r\n\r\n") {
            if body[h..].contains('}') {
                break;
            }
        }
        if body.len() > 64 * 1024 {
            break;
        }
    }

    let ip = json_field(&body, "query").ok_or_else(|| bad("no address in the reply"))?;
    let cc = json_field(&body, "countryCode").unwrap_or_default();
    Ok((ip, cc))
}

/// Pulls a flat string field out of a small JSON reply. Enough for the two
/// fields read here without pulling in a JSON parser.
pub fn json_field(body: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let at = body.find(&needle)? + needle.len();
    let rest = body[at..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
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
    tunnel: bool,
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
            tunnel: false,
        }
    }

    /// Whole-system mode: after the engine is up, sing-box routes every app
    /// through it. Off by default so the engine logic can be tested without a
    /// TUN adapter; the app turns it on.
    pub fn set_tunnel(&mut self, on: bool) {
        self.tunnel = on;
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

            // Absolute, because the child is started in another directory and a
            // relative program path would be looked up from there.
            let bin = std::path::absolute(&bin).unwrap_or(bin);
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

        if self.tunnel {
            if let Err(e) = self.start_tunnel(plan.socks) {
                self.stop();
                return Err(e);
            }
        }

        self.current = Some(engine);
        self.socks = Some(plan.socks);
        self.region = region;
        Ok(plan.socks)
    }

    fn start_tunnel(&mut self, socks_port: u16) -> io::Result<()> {
        let bin = tun_binary(&self.resources);
        if !bin.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("the tunnel core was not found at {}", bin.display()),
            ));
        }
        let dir = self.data.join("tunnel");
        fs::create_dir_all(&dir)?;
        let config = dir.join("config.json");
        fs::write(&config, tun_config(socks_port))?;

        let log = self.data.join("logs").join("tunnel.log");
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

        let bin = std::path::absolute(&bin).unwrap_or(bin);
        let config = std::path::absolute(&config).unwrap_or(config);
        let dir = std::path::absolute(&dir).unwrap_or(dir);
        let mut cmd = Command::new(&bin);
        cmd.arg("run")
            .arg("-c")
            .arg(&config)
            .arg("-D")
            .arg(&dir)
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn()?;

        // Creating the adapter normally takes a second or two. A failure (no
        // admin rights, adapter refused) makes sing-box exit, so an exit is
        // reported at once with the log rather than waited out.
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("the tunnel could not start; see {}", log.display()),
                ));
            }
            if port_is_open(TUN_API_PORT) {
                self.children.push(child);
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let _ = child.kill();
        let _ = child.wait();
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("the tunnel did not come up; see {}", log.display()),
        ))
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
    fn tun_config_points_at_the_engine_and_bypasses_the_cores() {
        let c = tun_config(1820);
        assert!(c.contains("\"server_port\": 1820"), "{c}");
        for p in TUN_BYPASS {
            assert!(c.contains(&format!("\"{p}\"")), "missing bypass for {p}");
        }
        assert!(c.contains(&format!("127.0.0.1:{TUN_API_PORT}")));
    }

    #[test]
    fn json_field_reads_the_exit_reply() {
        let body = "HTTP/1.1 200 OK\r\n\r\n{\"query\": \"203.0.113.7\",\"countryCode\":\"CA\"}";
        assert_eq!(json_field(body, "query").as_deref(), Some("203.0.113.7"));
        assert_eq!(json_field(body, "countryCode").as_deref(), Some("CA"));
        assert_eq!(json_field(body, "missing"), None);
    }

    /// A real SOCKS5 handshake against a stand-in proxy that forwards to a
    /// stand-in HTTP server, so the byte-level protocol is exercised end to end.
    #[test]
    fn exit_check_speaks_socks5_and_reads_the_answer() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let http = TcpListener::bind("127.0.0.1:0").unwrap();
        let http_port = http.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut c, _) = http.accept().unwrap();
            let mut req = String::new();
            let mut buf = [0u8; 256];
            while !req.contains("\r\n\r\n") {
                let n = c.read(&mut buf).unwrap();
                if n == 0 { break; }
                req.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            assert!(req.starts_with("GET /json/"), "{req}");
            c.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{\"query\":\"198.51.100.4\",\"countryCode\":\"DE\"}").unwrap();
        });

        let socks = TcpListener::bind("127.0.0.1:0").unwrap();
        let socks_port = socks.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut c, _) = socks.accept().unwrap();
            let mut g = [0u8; 3];
            c.read_exact(&mut g).unwrap();
            assert_eq!(g, [5, 1, 0]);
            c.write_all(&[5, 0]).unwrap();
            let mut h = [0u8; 5];
            c.read_exact(&mut h).unwrap();
            assert_eq!(&h[..4], &[5, 1, 0, 3]);
            let mut name = vec![0u8; h[4] as usize + 2];
            c.read_exact(&mut name).unwrap();
            assert_eq!(&name[..name.len() - 2], b"example.test");
            c.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();
            let mut up = TcpStream::connect(("127.0.0.1", http_port)).unwrap();
            let mut down = up.try_clone().unwrap();
            let mut c2 = c.try_clone().unwrap();
            std::thread::spawn(move || { let _ = std::io::copy(&mut c2, &mut up); });
            let _ = std::io::copy(&mut down, &mut c);
            let _ = c.shutdown(std::net::Shutdown::Both);
        });

        let (ip, cc) =
            exit_check_via(socks_port, "example.test", 80, "/json/?fields=query,countryCode").unwrap();
        assert_eq!(ip, "198.51.100.4");
        assert_eq!(cc, "DE");
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
