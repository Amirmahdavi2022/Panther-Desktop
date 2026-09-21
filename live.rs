use engcheck::*;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Every live test binds the same fixed engine ports, so they must not overlap.
/// Rust runs tests in parallel by default, which made them collide at random.
fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    match LOCK.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        // A previous test panicking must not poison the rest of the run.
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Waits for a port to go quiet after a kill, so the next test starts clean.
fn wait_until_closed(port: u16) {
    for _ in 0..40 {
        if !port_is_open(port) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Writes a stand-in core that really binds the port it is told to use, so the
/// supervisor is exercised against actual processes and actual sockets rather
/// than a mock.
fn fake_core(dir: &Path, stem: &str, reads: &str) {
    let bin = dir.join(stem);
    let mut f = fs::File::create(&bin).unwrap();
    writeln!(f, "#!/bin/sh").unwrap();
    match reads {
        // Aether style: --bind 127.0.0.1:PORT
        "bind" => {
            writeln!(f, "while [ $# -gt 0 ]; do if [ \"$1\" = \"--bind\" ]; then addr=$2; fi; shift; done").unwrap();
            writeln!(f, "port=$(echo \"$addr\" | cut -d: -f2)").unwrap();
        }
        // Global style: -config FILE, with the port inside the JSON
        _ => {
            writeln!(f, "while [ $# -gt 0 ]; do if [ \"$1\" = \"-config\" ]; then cfg=$2; fi; shift; done").unwrap();
            writeln!(f, "port=$(tr ',' '\\n' < \"$cfg\" | grep LocalSocksProxyPort | cut -d: -f2)").unwrap();
        }
    }
    writeln!(f, "exec python3 -c \"import socket,time;s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('127.0.0.1',int('$port')));s.listen(8);time.sleep(120)\"").unwrap();
    drop(f);
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
}

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("panther-live-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn supervisor_starts_a_real_process_and_sees_the_port() {
    let _guard = one_at_a_time();
    let dir = workspace("solo");
    fake_core(&dir, "aether", "bind");

    let mut sup = Supervisor::new(dir.clone(), dir.clone());
    assert!(!port_is_open(AETHER_PORT), "port must be free before the test");

    let socks = sup.start(Engine::Aether).expect("Aether should start and open its port");
    assert_eq!(socks, AETHER_PORT);
    assert!(sup.is_running());
    assert!(port_is_open(AETHER_PORT));
    assert_eq!(sup.current(), Some(Engine::Aether));

    sup.stop();
    assert!(!sup.is_running());
    wait_until_closed(AETHER_PORT);
    assert!(!port_is_open(AETHER_PORT), "the port must close after stop()");

    let _ = fs::remove_dir_all(&dir);
}

/// The Global engine is only useful chained behind Aether. This proves both
/// hops really come up, that the config the second hop reads is the one this
/// code wrote, and that the chain is torn down completely.
#[test]
fn global_brings_up_both_hops_and_serves_on_the_outer_one() {
    let _guard = one_at_a_time();
    let dir = workspace("chain");
    fake_core(&dir, "aether", "bind");
    fake_core(&dir, "global", "config");

    let mut sup = Supervisor::new(dir.clone(), dir.clone());
    let socks = sup.start(Engine::Global).expect("the chain should come up");

    assert_eq!(socks, GLOBAL_PORT, "callers must be handed the outer hop");
    assert!(port_is_open(AETHER_PORT), "the inner hop must be up");
    assert!(port_is_open(GLOBAL_PORT), "the outer hop must be up");
    assert!(sup.is_running());

    // The second hop found its port by reading the config this code generated,
    // so a config that did not carry the upstream would have failed above.
    let written = fs::read_to_string(dir.join("global-core").join("config.json")).unwrap();
    assert!(written.contains("socks5://127.0.0.1:1819"), "{written}");

    sup.stop();
    wait_until_closed(AETHER_PORT);
    wait_until_closed(GLOBAL_PORT);
    assert!(!port_is_open(AETHER_PORT), "stopping must take the inner hop down too");
    assert!(!port_is_open(GLOBAL_PORT), "stopping must take the outer hop down too");

    let _ = fs::remove_dir_all(&dir);
}

/// A half-started chain would look connected while carrying nothing, so a
/// missing second core must tear the first one down as well.
#[test]
fn a_missing_second_core_takes_the_whole_chain_down() {
    let _guard = one_at_a_time();
    let dir = workspace("partial");
    fake_core(&dir, "aether", "bind"); // global deliberately absent

    let mut sup = Supervisor::new(dir.clone(), dir.clone());
    let err = sup.start(Engine::Global).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);

    assert!(!sup.is_running(), "nothing may be left running");
    assert_eq!(sup.socks_port(), None);
    wait_until_closed(AETHER_PORT);
    assert!(!port_is_open(AETHER_PORT), "the first hop must not be left behind");

    let _ = fs::remove_dir_all(&dir);
}
