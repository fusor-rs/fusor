//! Serving published output on a host with no Cargo and no Rust.
mod support;

use std::fs;
use support::{Fixture, failure};

/// The manifest a build writes, with whatever fields a test needs to vary.
fn write_manifest(directory: &std::path::Path, base_path: &str) {
    fs::create_dir_all(directory).unwrap();
    fs::write(
        directory.join(".fusor-output.json"),
        format!(r#"{{"version":1,"generation":"g-1","base_path":"{base_path}"}}"#),
    )
    .unwrap();
}

#[test]
fn a_corrupt_base_path_is_rejected_before_the_server_starts() {
    let fixture = Fixture::new();
    write_manifest(&fixture.root.join("dist"), "/bad\\r\\nheader/");
    let error = failure(
        fixture
            .cli(&fixture.root, &["preview", "dist"])
            .env("PATH", ""),
    );
    assert!(error.contains("base-path"), "{error}");
}

#[test]
fn an_occupied_port_fails_with_the_flag_that_changes_it() {
    let fixture = Fixture::new();
    write_manifest(&fixture.root.join("dist"), "/");
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .expect("supported test hosts permit loopback listening");
    let port = listener.local_addr().unwrap().port().to_string();
    let error = failure(
        fixture
            .cli(&fixture.root, &["preview", "dist", "--port", &port])
            .env("PATH", ""),
    );
    assert!(error.contains("--port"), "{error}");
}

/// Recreate a browser opening new connections after closing its previous page.
/// Queue the burst while the server is stopped so the test does not depend on
/// how quickly the client can connect relative to the server's accept loop.
#[test]
#[cfg(target_os = "linux")]
fn connection_bursts_do_not_wait_for_other_keep_alive_sockets_to_close() {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::{Ipv4Addr, TcpListener, TcpStream},
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    struct Server(Child);
    impl Drop for Server {
        fn drop(&mut self) {
            // Also kills a stopped child if setup or an assertion fails.
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn request(stream: &mut BufReader<TcpStream>) {
        stream
            .get_mut()
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n")
            .unwrap();
    }

    fn response(stream: &mut BufReader<TcpStream>) {
        let mut line = String::new();
        stream
            .read_line(&mut line)
            .expect("response without closing other sockets");
        assert!(line.starts_with("HTTP/1.1 200 "), "{line}");
        let mut length = None;
        loop {
            line.clear();
            assert_ne!(stream.read_line(&mut line).unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = Some(value.trim().parse::<usize>().unwrap());
            }
        }
        let mut body = vec![0; length.expect("content length")];
        stream.read_exact(&mut body).unwrap();
        assert_eq!(body, b"ready");
    }

    let fixture = Fixture::new();
    let directory = fixture.root.join("dist");
    write_manifest(&directory, "/");
    fs::write(directory.join("index.html"), "ready").unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("supported test hosts permit loopback listening");
    let address = listener.local_addr().unwrap();
    drop(listener);

    // One CPU makes accept/worker scheduling reproducible even on large hosts.
    // Respect the test process's cpuset instead of assuming CPU 0 is available.
    let status = fs::read_to_string("/proc/self/status").unwrap();
    let cpu = status
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .unwrap()
        .trim()
        .split([',', '-'])
        .next()
        .unwrap();
    let mut server = Server(
        Command::new("taskset")
            .args(["-c", cpu, env!("CARGO_BIN_EXE_fusor"), "preview"])
            .arg(&directory)
            .args(["--port", &address.port().to_string(), "--quiet"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if TcpStream::connect(address).is_ok() {
            break;
        }
        assert!(server.0.try_wait().unwrap().is_none(), "preview exited");
        assert!(Instant::now() < deadline, "preview did not listen");
        thread::sleep(Duration::from_millis(10));
    }
    let connect = || {
        let stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        BufReader::new(stream)
    };
    let mut warm = Vec::new();
    for _ in 0..8 {
        let mut stream = connect();
        request(&mut stream);
        response(&mut stream);
        warm.push(stream);
    }
    drop(warm);
    thread::sleep(Duration::from_millis(100));
    let signal = |name: &str| {
        assert!(
            Command::new("kill")
                .args([name, &server.0.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
    };
    signal("-STOP");
    let mut burst: Vec<_> = (0..32)
        .map(|_| {
            let mut stream = connect();
            request(&mut stream);
            stream
        })
        .collect();
    signal("-CONT");
    // Keep every socket open while reading every response. A stranded request
    // must fail here; closing an earlier socket would hide the scheduling bug.
    for stream in &mut burst {
        response(stream);
    }
    request(&mut burst[0]);
    response(&mut burst[0]);
}
