//! The listener thread. Runs independently of the GUI, so the bridge keeps
//! working even if the window is minimised and not being drawn.

use std::io;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::caspar_in;
use crate::engine::{lock_engine, Engine};

const MAX_DATAGRAM: usize = 65_536;
/// Also the interval between timeout checks when no messages arrive.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub struct Worker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(port: u16, engine: Arc<Mutex<Engine>>) -> io::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        socket.set_read_timeout(Some(POLL_INTERVAL))?;

        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("caspar-osc-in".into())
                .spawn(move || run(socket, engine, stop))?
        };
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run(socket: UdpSocket, engine: Arc<Mutex<Engine>>, stop: Arc<AtomicBool>) {
    let mut buffer = vec![0u8; MAX_DATAGRAM];
    while !stop.load(Ordering::Relaxed) {
        let received = socket.recv_from(&mut buffer);
        let now = Instant::now();
        let mut engine = lock_engine(&engine);

        match received {
            Ok((len, _)) => {
                let updates = caspar_in::decode_updates(&buffer[..len], &|key| engine.watches(key));
                engine.apply(updates, now);
            }
            Err(error) if is_timeout(&error) => {}
            Err(_) => {
                drop(engine);
                thread::sleep(POLL_INTERVAL);
                continue;
            }
        }
        engine.refresh(now);
    }
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::{Binding, CountMode};
    use crate::companion_out::{CompanionSink, SinkError};
    use rosc::{encoder, OscMessage, OscPacket, OscType};

    struct RecordingSink(Arc<Mutex<Vec<String>>>);

    impl CompanionSink for RecordingSink {
        fn set_variable(&mut self, _: &str, text: &str) -> Result<(), SinkError> {
            self.0.lock().unwrap().push(text.to_owned());
            Ok(())
        }
    }

    fn send(socket: &UdpSocket, port: u16, addr: &str, args: Vec<OscType>) {
        let packet = OscPacket::Message(OscMessage {
            addr: addr.into(),
            args,
        });
        socket
            .send_to(&encoder::encode(&packet).unwrap(), ("127.0.0.1", port))
            .unwrap();
    }

    #[test]
    fn udp_in_to_sink_out_end_to_end() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let mut engine = Engine::new(vec![Binding {
            mode: CountMode::Down,
            variable: "timer1".into(),
            ..Binding::default()
        }]);
        engine.set_sink(Some(Box::new(RecordingSink(Arc::clone(&sent)))));
        let engine = Arc::new(Mutex::new(engine));

        // Find a free port by binding and releasing it again.
        let port = UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let worker = Worker::start(port, Arc::clone(&engine)).unwrap();

        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        send(
            &sender,
            port,
            "/channel/1/stage/layer/10/foreground/file/name",
            vec![OscType::String("AMB".into())],
        );
        send(
            &sender,
            port,
            "/channel/1/stage/layer/10/foreground/file/time",
            vec![OscType::Float(2.0), OscType::Float(10.0)],
        );
        // A layer without a binding:
        send(
            &sender,
            port,
            "/channel/1/stage/layer/11/foreground/file/name",
            vec![OscType::String("IGNORED".into())],
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        while sent.lock().unwrap().last().map(String::as_str) != Some("AMB 00:08")
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(20));
        }
        drop(worker);

        assert_eq!(
            sent.lock().unwrap().last().map(String::as_str),
            Some("AMB 00:08")
        );
        assert!(!engine
            .lock()
            .unwrap()
            .watches(crate::layer_state::LayerKey {
                channel: 1,
                layer: 11
            }));
    }
}
