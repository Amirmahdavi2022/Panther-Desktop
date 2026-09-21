use engcheck::*;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

/// Spawns a stand-in engine that really binds the SOCKS port, to prove the
/// supervisor's start -> wait-for-port -> running path works end to end.
#[test]
fn supervisor_starts_a_real_process_and_sees_the_port() {
    let dir = PathBuf::from(format!("/tmp/eng-live-{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let bin = dir.join("aether");

    let mut f = fs::File::create(&bin).unwrap();
    // Reads "--bind 127.0.0.1:PORT" exactly as Engine::args emits it.
    writeln!(f, "#!/bin/sh").unwrap();
    // Walk the args to find the value after --bind, exactly as a real core would.
    writeln!(f, "while [ $# -gt 0 ]; do if [ \"$1\" = \"--bind\" ]; then addr=$2; fi; shift; done").unwrap();
    writeln!(f, "port=$(echo \"$addr\" | cut -d: -f2)").unwrap();
    writeln!(f, "exec python3 -c \"import socket,time;s=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('127.0.0.1',int('$port')));s.listen(8);time.sleep(60)\"").unwrap();
    drop(f);
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();

    let port = 18099;
    let mut sup = Supervisor::new(dir.clone());

    assert!(!port_is_open(port), "port must be free before the test");
    sup.start(Engine::Aether, port).expect("engine should start and open the port");
    assert!(sup.is_running(), "supervisor should report the child alive");
    assert!(port_is_open(port), "the port must be open once start() returns");
    assert_eq!(sup.current(), Some(Engine::Aether));

    sup.stop();
    assert!(!sup.is_running(), "supervisor should report stopped");
    // give the OS a moment to release the socket
    std::thread::sleep(Duration::from_millis(600));
    assert!(!port_is_open(port), "the port must be closed after stop()");

    let _ = fs::remove_dir_all(&dir);
}
