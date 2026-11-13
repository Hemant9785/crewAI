use std::convert::TryFrom;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

trait Transport {
    fn send_message(&mut self, message: &[u8]) -> io::Result<()>;
    fn receive_message(&mut self) -> io::Result<Vec<u8>>;
}

struct TcpTransport {
    stream: TcpStream,
}

impl TcpTransport {
    fn new(stream: TcpStream) -> Self {
        Self { stream }
    }
}

impl Transport for TcpTransport {
    fn send_message(&mut self, message: &[u8]) -> io::Result<()> {
        let length = u32::try_from(message.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "message too large"))?;
        self.stream.write_all(&length.to_be_bytes())?;
        self.stream.write_all(message)
    }

    fn receive_message(&mut self) -> io::Result<Vec<u8>> {
        let mut length = [0; 4];
        self.stream.read_exact(&mut length)?;
        let length = u32::from_be_bytes(length) as usize;
        let mut message = vec![0; length];
        self.stream.read_exact(&mut message)?;
        Ok(message)
    }
}

fn main() -> io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let address = listener.local_addr()?;

    let server = thread::spawn(move || -> io::Result<()> {
        let (stream, _) = listener.accept()?;
        let mut transport = TcpTransport::new(stream);
        let message = transport.receive_message()?;
        if message != b"phase-0" {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "unexpected message"));
        }
        transport.send_message(b"ack:phase-0")
    });

    let mut client = TcpTransport::new(TcpStream::connect(address)?);
    client.send_message(b"phase-0")?;
    let response = client.receive_message()?;
    assert_eq!(response, b"ack:phase-0");
    server.join().expect("server thread panicked")?;
    println!("transport trait loopback passed");
    Ok(())
}
