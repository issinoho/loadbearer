//! Network benchmark: loopback TCP and UDP over `127.0.0.1`, measuring the
//! machine's network *stack* — syscall overhead, TCP processing, and
//! scheduler wakeup latency — not any physical link. Nothing leaves the machine.
//!
//! ## Methodology
//!
//! Each subtest spins up a server on a background thread bound to an ephemeral
//! `127.0.0.1` port, then a client thread drives the measurement for the
//! subtest's time budget. Server threads are torn down (connection close, or a
//! stop flag for the connectionless UDP case) before the function returns.
//!
//! - **tcp_stream** — one connection, blast 256 KiB writes; the rate is gated by
//!   the reader thread draining the socket, so it's real end-to-end loopback
//!   throughput.
//! - **tcp_parallel** — the same on every logical CPU at once, rates summed.
//! - **tcp_rtt** — 64-byte request/response ping-pong; one message in flight, so
//!   it measures the full syscall + loopback + scheduler round trip.
//! - **udp_pps** — blast 64-byte datagrams and count the successful `send`
//!   syscalls per second (per-packet overhead). Failed sends are not counted.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::engine::{Benchmark, Direction, RunContext, SubtestSpec, parallel_sum, throughput};
use crate::scoring::LinkResult;
use crate::util::GIB;

pub struct NetworkBenchmark;

/// Payload size for the streaming subtests.
const STREAM_CHUNK: usize = 256 * 1024;
/// Message size for the latency and packet-rate subtests.
const MSG: usize = 64;
const LOCALHOST: &str = "127.0.0.1:0";

impl Benchmark for NetworkBenchmark {
    fn id(&self) -> &'static str {
        "network"
    }

    fn label(&self) -> &'static str {
        "Network"
    }

    fn subtests(&self) -> Vec<SubtestSpec> {
        use Direction::{HigherIsBetter as Hi, LowerIsBetter as Lo};
        vec![
            SubtestSpec {
                id: "tcp_stream",
                label: "TCP throughput, single stream",
                unit: "GiB/s",
                direction: Hi,
            },
            SubtestSpec {
                id: "tcp_parallel",
                label: "TCP throughput, all streams",
                unit: "GiB/s",
                direction: Hi,
            },
            SubtestSpec {
                id: "tcp_rtt",
                label: "TCP round-trip latency",
                unit: "us",
                direction: Lo,
            },
            SubtestSpec {
                id: "udp_pps",
                label: "UDP send rate",
                unit: "Kpps",
                direction: Hi,
            },
        ]
    }

    fn run_subtest(&self, subtest_id: &str, ctx: &RunContext) -> Result<f64> {
        let budget = ctx.time_budget();
        Ok(match subtest_id {
            "tcp_stream" => tcp_stream(budget)?,
            "tcp_parallel" => tcp_parallel(budget, ctx.threads)?,
            "tcp_rtt" => tcp_rtt(budget)?,
            "udp_pps" => udp_pps(budget)?,
            other => bail!("unknown network subtest: {other}"),
        })
    }
}

/// Drain a TCP connection until the peer closes it.
fn drain_tcp(mut conn: TcpStream) {
    let mut buf = vec![0u8; STREAM_CHUNK];
    while matches!(conn.read(&mut buf), Ok(n) if n > 0) {}
}

/// Blast `STREAM_CHUNK` writes down `stream` for `budget`; return GiB/s. A write
/// failing near the boundary (the peer tearing the connection down as time runs
/// out) is tolerated — the loop just stops.
fn blast(mut stream: TcpStream, budget: Duration) -> f64 {
    let payload = vec![0xA5u8; STREAM_CHUNK];
    let start = std::time::Instant::now();
    let mut bytes = 0u64;
    while start.elapsed() < budget {
        match stream.write_all(&payload) {
            Ok(()) => bytes += STREAM_CHUNK as u64,
            Err(_) => break,
        }
    }
    bytes as f64 / start.elapsed().as_secs_f64() / GIB
}

fn tcp_stream(budget: Duration) -> Result<f64> {
    let listener = TcpListener::bind(LOCALHOST)?;
    let addr = listener.local_addr()?;
    let server = thread::spawn(move || {
        if let Ok((conn, _)) = listener.accept() {
            drain_tcp(conn);
        }
    });

    let stream = TcpStream::connect(addr)?;
    stream.set_nodelay(true)?;
    let gibps = blast(stream, budget);

    let _ = server.join();
    Ok(gibps)
}

fn tcp_parallel(budget: Duration, streams: usize) -> Result<f64> {
    let streams = streams.max(1);
    let listener = TcpListener::bind(LOCALHOST)?;
    let addr = listener.local_addr()?;

    // The acceptor knows exactly how many connections to expect, so a plain
    // blocking accept loop is enough — no nonblocking / stop-flag dance.
    let acceptor = thread::spawn(move || {
        let mut drains = Vec::with_capacity(streams);
        for _ in 0..streams {
            match listener.accept() {
                Ok((conn, _)) => drains.push(thread::spawn(move || drain_tcp(conn))),
                Err(_) => break,
            }
        }
        for d in drains {
            let _ = d.join();
        }
    });

    let total = parallel_sum(streams, || {
        let Ok(stream) = TcpStream::connect(addr) else {
            return 0.0;
        };
        stream.set_nodelay(true).ok();
        blast(stream, budget)
    });

    let _ = acceptor.join();
    Ok(total)
}

fn tcp_rtt(budget: Duration) -> Result<f64> {
    let listener = TcpListener::bind(LOCALHOST)?;
    let addr = listener.local_addr()?;
    let server = thread::spawn(move || {
        if let Ok((mut conn, _)) = listener.accept() {
            conn.set_nodelay(true).ok();
            let mut buf = [0u8; MSG];
            while conn.read_exact(&mut buf).is_ok() {
                if conn.write_all(&buf).is_err() {
                    break;
                }
            }
        }
    });

    let mut stream = TcpStream::connect(addr)?;
    stream.set_nodelay(true)?;
    let mut buf = [0u8; MSG];
    let round_trips_per_sec = throughput(budget, || {
        match (stream.write_all(&buf), stream.read_exact(&mut buf)) {
            (Ok(()), Ok(())) => 1,
            _ => 0,
        }
    });

    drop(stream);
    let _ = server.join();
    if round_trips_per_sec <= 0.0 {
        anyhow::bail!("no TCP round trips completed on loopback");
    }
    Ok(1_000_000.0 / round_trips_per_sec)
}

fn udp_pps(budget: Duration) -> Result<f64> {
    let server = UdpSocket::bind(LOCALHOST)?;
    server.set_nonblocking(true)?;
    let addr = server.local_addr()?;

    let stop = Arc::new(AtomicBool::new(false));
    let drain_stop = stop.clone();
    let drain = thread::spawn(move || {
        let mut buf = [0u8; 2048];
        while !drain_stop.load(Ordering::Relaxed) {
            // Drain everything queued, then re-check the stop flag.
            while server.recv(&mut buf).is_ok() {}
        }
    });

    let client = UdpSocket::bind(LOCALHOST)?;
    client.connect(addr)?;
    let payload = [0xA5u8; MSG];
    let sent_per_sec = throughput(budget, || {
        let mut ok = 0u64;
        for _ in 0..16 {
            if client.send(&payload).is_ok() {
                ok += 1;
            }
        }
        ok
    });

    stop.store(true, Ordering::Relaxed);
    let _ = drain.join();
    Ok(sent_per_sec / 1000.0)
}

// ---------------------------------------------------------------------------
// Optional link test: `loadbearer net-server` on one machine, `loadbearer run
// --net-target HOST:PORT` on another. Measures the real path between them. Not
// scored against the baseline — it's a property of the network, not the host.
// ---------------------------------------------------------------------------

/// First byte a client sends on a TCP connection to a `net-server`, selecting
/// what that connection is for.
const MODE_SINK: u8 = b'S';
const MODE_ECHO: u8 = b'E';

/// Probe a running `net-server` at `target` (`host:port`). Each of the three
/// measurements runs for `budget`.
pub fn link_probe(target: &str, budget: Duration) -> Result<LinkResult> {
    // Upload throughput.
    let mut up = TcpStream::connect(target)?;
    up.set_nodelay(true)?;
    up.write_all(&[MODE_SINK])?;
    let up_gibps = blast(up, budget);

    // Round-trip latency.
    let mut rt = TcpStream::connect(target)?;
    rt.set_nodelay(true)?;
    rt.write_all(&[MODE_ECHO])?;
    let mut buf = [0u8; MSG];
    let rtts = throughput(budget, || {
        match (rt.write_all(&buf), rt.read_exact(&mut buf)) {
            (Ok(()), Ok(())) => 1,
            _ => 0,
        }
    });
    drop(rt);

    // UDP send rate.
    let udp = UdpSocket::bind("0.0.0.0:0")?;
    udp.connect(target)?;
    let dgram = [0xA5u8; MSG];
    let sends = throughput(budget, || {
        let mut ok = 0u64;
        for _ in 0..16 {
            if udp.send(&dgram).is_ok() {
                ok += 1;
            }
        }
        ok
    });

    anyhow::ensure!(rtts > 0.0, "no round trips completed to {target}");
    Ok(LinkResult {
        target: target.to_string(),
        tcp_upload_gibps: up_gibps,
        tcp_rtt_us: 1_000_000.0 / rtts,
        udp_send_kpps: sends / 1000.0,
    })
}

/// Run the server side of the link test until the process is killed. Handles
/// each TCP connection by its mode byte (sink or echo), and drains a UDP socket
/// on the same port.
pub fn serve(bind: &str) -> Result<()> {
    let tcp = TcpListener::bind(bind)?;
    let local = tcp.local_addr()?;
    let udp = UdpSocket::bind(local)?;

    thread::spawn(move || {
        let mut buf = [0u8; 2048];
        loop {
            let _ = udp.recv(&mut buf);
        }
    });

    println!("loadbearer net-server listening on {local} (TCP + UDP)");
    println!("run `loadbearer run --net-target {local}` from another machine; Ctrl-C to stop");

    for conn in tcp.incoming() {
        let Ok(mut conn) = conn else { continue };
        thread::spawn(move || {
            conn.set_nodelay(true).ok();
            let mut mode = [0u8; 1];
            if conn.read_exact(&mut mode).is_err() {
                return;
            }
            match mode[0] {
                MODE_ECHO => {
                    let mut buf = [0u8; MSG];
                    while conn.read_exact(&mut buf).is_ok() {
                        if conn.write_all(&buf).is_err() {
                            break;
                        }
                    }
                }
                _ => drain_tcp(conn), // MODE_SINK and anything else
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_subtest_yields_a_positive_value() {
        let b = Duration::from_millis(20);
        assert!(tcp_stream(b).unwrap() > 0.0);
        assert!(tcp_parallel(b, 2).unwrap() > 0.0);
        assert!(tcp_rtt(b).unwrap() > 0.0);
        assert!(udp_pps(b).unwrap() > 0.0);
    }

    #[test]
    fn rejects_unknown_subtest() {
        let ctx = RunContext {
            preset: crate::engine::DurationPreset::Short,
            seed: 1,
            target_dir: std::env::temp_dir(),
            threads: 2,
            total_ram: 8 << 30,
            runs_override: None,
            abort: Arc::new(AtomicBool::new(false)),
        };
        assert!(NetworkBenchmark.run_subtest("nope", &ctx).is_err());
    }

    #[test]
    fn link_probe_talks_to_a_local_server() {
        // A minimal stand-in for `serve`, on an ephemeral port.
        let listener = TcpListener::bind(LOCALHOST).unwrap();
        let addr = listener.local_addr().unwrap();
        let udp = UdpSocket::bind(addr).unwrap();
        thread::spawn(move || {
            let mut b = [0u8; 2048];
            loop {
                let _ = udp.recv(&mut b);
            }
        });
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut conn) = conn else { continue };
                thread::spawn(move || {
                    conn.set_nodelay(true).ok();
                    let mut mode = [0u8; 1];
                    if conn.read_exact(&mut mode).is_err() {
                        return;
                    }
                    if mode[0] == MODE_ECHO {
                        let mut buf = [0u8; MSG];
                        while conn.read_exact(&mut buf).is_ok() {
                            if conn.write_all(&buf).is_err() {
                                break;
                            }
                        }
                    } else {
                        drain_tcp(conn);
                    }
                });
            }
        });

        let result = link_probe(&addr.to_string(), Duration::from_millis(20)).unwrap();
        assert!(result.tcp_upload_gibps > 0.0);
        assert!(result.tcp_rtt_us > 0.0);
        assert!(result.udp_send_kpps > 0.0);
        assert_eq!(result.target, addr.to_string());
    }
}
