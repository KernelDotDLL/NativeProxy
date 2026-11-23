use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;
use std::{io, thread};
//Warning: Since im drunk and i've read the RFC 1928 like a moron. this shit can be unstable. shits gonna change very fast

const ADDR: &str = "127.0.0.1:25567";
const SOCKS_VERSION: u8 = 0x05;
const METHOD_NO_AUTH: u8 = 0x00;
const METHOD_NO_ACCEPTABLE: u8 = 0xFF;
const CMD_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;
const REP_SUCCEEDED: u8 = 0x00;
const REP_GENERAL_FAILURE: u8 = 0x01;
const REP_NETWORK_UNREACHABLE: u8 = 0x03;
const REP_HOST_UNREACHABLE: u8 = 0x04;
const REP_CONNECTION_REFUSED: u8 = 0x05;
const REP_TTL_EXPIRED: u8 = 0x06;
const REP_CMD_NOT_SUPPORTED: u8 = 0x07;
const REP_ADDR_NOT_SUPPORTED: u8 = 0x08;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

fn main() {
    let listener = match TcpListener::bind(ADDR) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind to {ADDR}: {e}");
            return;
        }
    };

    println!("SOCKS5 server listening on {ADDR}");

    loop {
        match listener.accept() {
            Ok((stream, peer)) => {
                thread::spawn(move || {
                    if let Err(e) = handle_client(stream) {
                        eprintln!("client {peer}: {e}");
                    }
                });
            }
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::ConnectionAborted | ErrorKind::Interrupted
                ) => {}
            Err(e) => {
                eprintln!("accept failed: {e}");
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn handle_client(mut client: TcpStream) -> io::Result<()> {
    client.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    client.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
    let _ = client.set_nodelay(true);

    let mut greeting = [0u8; 2];
    client.read_exact(&mut greeting)?;
    if greeting[0] != SOCKS_VERSION {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!("bad SOCKS version: {}", greeting[0]),
        ));
    }

    //ok now reading the methods
    let mut methods = vec![0u8; greeting[1] as usize];
    client.read_exact(&mut methods)?;

    if !methods.contains(&METHOD_NO_AUTH) {
        client.write_all(&[SOCKS_VERSION, METHOD_NO_ACCEPTABLE])?;
        return Ok(());
    }
    client.write_all(&[SOCKS_VERSION, METHOD_NO_AUTH])?;

    //now request header which is [VER, CMD, RSV, ATYP]
    let mut req = [0u8; 4];
    client.read_exact(&mut req)?;

    if req[0] != SOCKS_VERSION {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "bad request version",
        ));
    }

    let (cmd, atyp) = (req[1], req[3]);

    if cmd != CMD_CONNECT {
        send_reply(&mut client, REP_CMD_NOT_SUPPORTED, None)?;
        return Ok(());
    }

    //now lets parse the target address (according to the fucking atyp)
    let target_addr: Vec<SocketAddr> = match parse_target(&mut client, atyp) {
        Ok(addr) => addr,
        Err(e) if e.kind() == ErrorKind::Unsupported => {
            send_reply(&mut client, REP_ADDR_NOT_SUPPORTED, None)?;
            return Ok(());
        }
        Err(e) => {
            send_reply(&mut client, rep_for_io_error(&e), None)?;
            return Err(e);
        }
    };

    //ok now that we read all of that. isn't it time to connect and replay? surely is right? spoiler alert: no

    let outbound = match connect_first(&target_addr, CONNECT_TIMEOUT) {
        Ok(s) => s,
        Err(e) => {
            send_reply(&mut client, rep_for_io_error(&e), None)?;
            return Ok(());
        }
    };

    let _ = outbound.set_nodelay(true);
    let bound = outbound.local_addr().ok();
    send_reply(&mut client, REP_SUCCEEDED, bound)?;

    let _ = client.set_read_timeout(None);
    let _ = client.set_write_timeout(None);

    let _ = outbound.set_read_timeout(None);
    let _ = outbound.set_write_timeout(None);

    relay(client, outbound);

    Ok(())
}

fn connect_first(target_addr: &[SocketAddr], timeout: Duration) -> io::Result<TcpStream> {
    let mut last = io::Error::new(ErrorKind::AddrNotAvailable, "no target addresses");
    for addr in target_addr {
        match TcpStream::connect_timeout(addr, timeout) {
            Ok(stream) => return Ok(stream),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn rep_for_io_error(e: &io::Error) -> u8 {
    match e.kind() {
        ErrorKind::NetworkUnreachable => REP_NETWORK_UNREACHABLE,
        ErrorKind::HostUnreachable | ErrorKind::NotFound => REP_HOST_UNREACHABLE,
        ErrorKind::ConnectionRefused => REP_CONNECTION_REFUSED,
        ErrorKind::TimedOut => REP_TTL_EXPIRED,
        ErrorKind::AddrNotAvailable | ErrorKind::Unsupported | ErrorKind::InvalidData => {
            REP_ADDR_NOT_SUPPORTED
        }
        _ => REP_GENERAL_FAILURE,
    }
}

fn parse_target<R: Read>(reader: &mut R, atyp: u8) -> io::Result<Vec<SocketAddr>> {
    match atyp {
        ATYP_IPV4 => {
            let mut addr_buf = [0u8; 4];
            let mut port_buf = [0u8; 2];
            reader.read_exact(&mut addr_buf)?;
            reader.read_exact(&mut port_buf)?;

            let port = u16::from_be_bytes(port_buf);
            let ip = std::net::Ipv4Addr::from(addr_buf);
            Ok(vec![SocketAddr::from((ip, port))])
        }
        ATYP_IPV6 => {
            let mut addr_buf = [0u8; 16];
            let mut port_buf = [0u8; 2];
            reader.read_exact(&mut addr_buf)?;
            reader.read_exact(&mut port_buf)?;
            let port = u16::from_be_bytes(port_buf);
            let ip = std::net::Ipv6Addr::from(addr_buf);
            Ok(vec![SocketAddr::from((ip, port))])
        }
        ATYP_DOMAIN => {
            let mut len_buf = [0u8; 1];
            reader.read_exact(&mut len_buf)?;
            let len = len_buf[0] as usize;
            if len == 0 {
                return Err(io::Error::new(ErrorKind::Unsupported, "empty domain"));
            }

            let mut domain_buf = vec![0u8; len];
            reader.read_exact(&mut domain_buf)?;

            let mut port_buf = [0u8; 2];
            reader.read_exact(&mut port_buf)?;
            let port = u16::from_be_bytes(port_buf);

            let host = std::str::from_utf8(&domain_buf)
                .map_err(|_| io::Error::new(ErrorKind::InvalidData, "domain is not UTF-8"))?;
            let host = host.strip_suffix('.').unwrap_or(host);

            let addrs: Vec<SocketAddr> = (host, port)
                .to_socket_addrs()
                .map_err(|e| io::Error::new(ErrorKind::NotFound, e))?
                .collect();
            if addrs.is_empty() {
                return Err(io::Error::new(
                    ErrorKind::NotFound,
                    "domain resolved to nothing",
                ));
            }
            Ok(addrs)
        }
        _ => Err(io::Error::new(
            ErrorKind::Unsupported,
            format!("unsupported ATYP: {atyp:#x}"),
        )),
    }
}

fn relay(client: TcpStream, outbound: TcpStream) {
    let mut client_to_remote = match client.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("relay: cannot clone client socket: {e}");
            return;
        }
    };
    let mut remote_to_client = match outbound.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("relay: cannot clone target socket: {e}");
            return;
        }
    };

    let mut outbound_writer = outbound;
    let mut client_writer = client;

    let t1 = thread::spawn(move || {
        if let Err(e) = io::copy(&mut client_to_remote, &mut outbound_writer) {
            eprintln!("relay client->target: {e}");
        }
        let _ = outbound_writer.shutdown(Shutdown::Write);
    });

    let t2 = thread::spawn(move || {
        if let Err(e) = io::copy(&mut remote_to_client, &mut client_writer) {
            eprintln!("relay target->client: {e}");
        }
        let _ = client_writer.shutdown(Shutdown::Write);
    });

    let _ = t1.join();
    let _ = t2.join();
}

fn encode_reply(rep: u8, bound: Option<SocketAddr>) -> Vec<u8> {
    let mut reply = Vec::with_capacity(32);
    reply.extend_from_slice(&[SOCKS_VERSION, rep, 0x00]);
    match bound {
        Some(SocketAddr::V4(a)) => {
            reply.push(ATYP_IPV4);
            reply.extend_from_slice(&a.ip().octets());
            reply.extend_from_slice(&a.port().to_be_bytes());
        }
        Some(SocketAddr::V6(a)) => {
            reply.push(ATYP_IPV6);
            reply.extend_from_slice(&a.ip().octets());
            reply.extend_from_slice(&a.port().to_be_bytes());
        }
        None => {
            reply.push(ATYP_IPV4);
            reply.extend_from_slice(&[0u8; 6]);
        }
    }
    reply
}
fn send_reply(client: &mut TcpStream, rep: u8, bound: Option<SocketAddr>) -> io::Result<()> {
    client.write_all(&encode_reply(rep, bound))
}