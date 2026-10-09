//! Gateway to Companion. The rest of the program only knows `CompanionSink`.

use std::fmt;
use std::net::{ToSocketAddrs, UdpSocket};

use rosc::{encoder, OscMessage, OscPacket, OscType};

#[derive(Debug)]
pub struct SinkError(String);

impl fmt::Display for SinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SinkError {}

pub trait CompanionSink {
    /// `name` must already be a valid variable name (see `binding::is_valid_variable_name`).
    fn set_variable(&mut self, name: &str, text: &str) -> Result<(), SinkError>;
}

/// Sends `/custom-variable/<name>/value <text>` to Companion's OSC port.
pub struct UdpCompanionSink {
    socket: UdpSocket,
}

impl UdpCompanionSink {
    pub fn connect(host: &str, port: u16) -> Result<Self, SinkError> {
        let address = (host, port)
            .to_socket_addrs()
            .map_err(|e| SinkError(format!("Cannot resolve {host}:{port}: {e}")))?
            .next()
            .ok_or_else(|| SinkError(format!("No address found for {host}:{port}")))?;
        let local = if address.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
        let socket = UdpSocket::bind(local)
            .map_err(|e| SinkError(format!("Cannot open UDP socket: {e}")))?;
        socket
            .connect(address)
            .map_err(|e| SinkError(format!("Cannot use {address}: {e}")))?;
        Ok(Self { socket })
    }
}

impl CompanionSink for UdpCompanionSink {
    fn set_variable(&mut self, name: &str, text: &str) -> Result<(), SinkError> {
        let bytes = encode_variable_message(name, text)?;
        self.socket
            .send(&bytes)
            .map(|_| ())
            .map_err(|e| SinkError(format!("Could not send to Companion: {e}")))
    }
}

fn osc_address(variable: &str) -> String {
    format!("/custom-variable/{variable}/value")
}

fn encode_variable_message(variable: &str, text: &str) -> Result<Vec<u8>, SinkError> {
    let packet = OscPacket::Message(OscMessage {
        addr: osc_address(variable),
        args: vec![OscType::String(text.to_owned())],
    });
    encoder::encode(&packet).map_err(|e| SinkError(format!("Could not encode OSC: {e:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_companion_address() {
        assert_eq!(osc_address("timer1"), "/custom-variable/timer1/value");
    }

    #[test]
    fn text_is_an_argument_not_part_of_the_address() {
        let bytes = encode_variable_message("timer1", "AMB - 00:07").unwrap();
        let (_, packet) = rosc::decoder::decode_udp(&bytes).unwrap();
        let OscPacket::Message(message) = packet else {
            panic!("expected a message");
        };
        assert_eq!(message.addr, "/custom-variable/timer1/value");
        assert_eq!(message.args, vec![OscType::String("AMB - 00:07".into())]);
    }

    #[test]
    fn udp_sink_delivers_to_listener() {
        let listener = UdpSocket::bind("127.0.0.1:0").unwrap();
        listener
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let port = listener.local_addr().unwrap().port();

        let mut sink = UdpCompanionSink::connect("127.0.0.1", port).unwrap();
        sink.set_variable("timer1", "hello").unwrap();

        let mut buf = [0u8; 512];
        let len = listener.recv(&mut buf).unwrap();
        let (_, packet) = rosc::decoder::decode_udp(&buf[..len]).unwrap();
        let OscPacket::Message(message) = packet else {
            panic!("expected a message");
        };
        assert_eq!(message.addr, "/custom-variable/timer1/value");
        assert_eq!(message.args, vec![OscType::String("hello".into())]);
    }
}
