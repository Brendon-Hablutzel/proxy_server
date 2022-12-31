use httparse;
use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    thread,
};
use url;

macro_rules! err_to_str {
    ($fallible:expr) => {
        $fallible.map_err(|err| err.to_string())
    };
}

enum PipeError {
    SocketClosed,
    Unknown(String),
}

fn pipe(
    in_stream: &mut TcpStream,
    out_stream: &mut TcpStream,
    buffer: &mut [u8],
    mut bytes_to_pass: usize,
) -> Result<usize, PipeError> {
    if bytes_to_pass == 0 {
        bytes_to_pass = match in_stream.read(buffer) {
            Ok(0) => return Err(PipeError::SocketClosed), // socket has been closed
            Ok(bytes) => bytes, // number of bytes successfully read into the buffer
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock => 0, // no data ready to be read
                _ => return Err(PipeError::Unknown(format!("{e}"))),
            },
        }
    }

    if bytes_to_pass > 0 {
        match out_stream.write(&buffer[0..bytes_to_pass]) {
            Ok(0) => return Err(PipeError::SocketClosed), // socket has been closed
            Ok(bytes) if bytes == bytes_to_pass => bytes_to_pass = 0, // all data in buffer has been written to stream; buffer is ready to be refilled
            Ok(_) => {
                return Err(PipeError::Unknown(
                    "Unable to write all bytes to stream".to_owned(),
                ))
            }
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock => (), // wait to write until a later call
                _ => return Err(PipeError::Unknown(format!("{e}"))),
            },
        }
    }
    // returns number of bytes in `buffer` that must still be written to `out_stream`
    Ok(bytes_to_pass)
}

fn handle_connection(mut client_stream: TcpStream) -> Result<(), String> {
    let mut initial_request_buffer = [0; 1024];
    err_to_str!(client_stream.read(&mut initial_request_buffer))?;

    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut request = httparse::Request::new(&mut headers);
    let _result = err_to_str!(request.parse(&initial_request_buffer))?;

    let method = request.method.ok_or("Unable to get request method")?;
    let path = request.path.ok_or("Unable to get request path")?;

    let mut server_stream = if method == "CONNECT" {
        println!("Connecting securely...");
        let stream = err_to_str!(TcpStream::connect(path))?;

        err_to_str!(client_stream.write(b"HTTP/1.1 200 OK\r\n\r\n"))?;

        stream
    } else {
        println!("Connecting via http...");
        let path = err_to_str!(url::Url::parse(path))?;
        let addr = err_to_str!(path.socket_addrs(|| Some(80)))?;
        let addr = addr
            .get(0)
            .ok_or("Unable to parse url into socket address")?;

        let mut stream = err_to_str!(TcpStream::connect(addr))?;
        err_to_str!(stream.write(&initial_request_buffer))?;
        stream
    };

    // client_stream.flush().unwrap(); not sure if this is needed

    err_to_str!(client_stream.set_nonblocking(true))?;
    err_to_str!(server_stream.set_nonblocking(true))?;

    // bytes read from client to be written to server
    let mut client_buffer = [0; 4096];
    // bytes read from server to be written to client
    let mut server_buffer = [0; 4096];

    // num bytes in `client_buffer` to write to `server_stream`
    let mut to_write_to_server = 0;
    // num bytes in `server_buffer` to write to `client_stream`
    let mut to_write_to_client = 0;

    loop {
        to_write_to_server = match pipe(
            &mut client_stream,
            &mut server_stream,
            &mut client_buffer,
            to_write_to_server,
        ) {
            Ok(bytes) => bytes,
            Err(PipeError::SocketClosed) => return Err("Client socket closed".to_owned()),
            Err(PipeError::Unknown(e)) => return Err(e),
        };

        to_write_to_client = match pipe(
            &mut server_stream,
            &mut client_stream,
            &mut server_buffer,
            to_write_to_client,
        ) {
            Ok(bytes) => bytes,
            Err(PipeError::SocketClosed) => return Err("Server socket closed".to_owned()),
            Err(PipeError::Unknown(e)) => return Err(e),
        };
    }
}

fn main() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:8080")
        .map_err(|err| format!("Could not start TCP listener: {err}"))?;

    println!("Server started...");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                thread::spawn(move || {
                    if let Err(e) = handle_connection(stream) {
                        eprintln!("{e}")
                    }
                });
            }
            Err(e) => eprintln!("Could not connect to stream: {e}"),
        }
    }

    Ok(())
}
