use std::{io, thread};
use std::fmt::format;
use std::fs::ReadDir;
use std::io::{ErrorKind, Read, Write};
use std::io::ErrorKind::NotFound;
use std::net::{Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream, ToSocketAddrs};


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
const REP_CMD_NOT_SUPPORTED: u8 = 0x07;
const REP_ADDR_NOT_SUPPORTED: u8 = 0x08;


fn main() {


    let listener = match TcpListener::bind(ADDR) {
        Ok(l) => l,
        Err(e) if e.kind() == ErrorKind::AddrInUse => {
            eprintln!("Cannot bind to {ADDR}: the port is already in use.");
            return;
        },
        Err(e) => {
            eprintln!("Failed to bind to {ADDR}: {e}");
            return;
        }
    };

    println!("SOCKS5 server listening on {ADDR}");

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                thread::spawn(move || {
                    if let Err(e) = handle_client(stream) {
                        eprintln!("client error: {e}");
                    }
                });
            }
            Err(e) => {
                println!("Accept error: {e}");
            }
        }
    }
}

fn handle_client(mut client: TcpStream) -> io::Result<()> {
    //alr i'm gonna read the Handshake first. which includes version and method list
    let mut header = [0u8; 2];
    client.read_exact(&mut header)?;
    if header[0] != SOCKS_VERSION {
        return Err(io::Error::new(ErrorKind::InvalidData, format!("Invalid SOCKS version: {}", header[0])));
    }

    //ok now reading the methods
    let methods = header[1] as usize;
    let mut methods = vec![0u8; methods];
    client.read_exact(&mut methods)?;

    if !methods.contains(&METHOD_NO_AUTH) {
        client.write_all(&[SOCKS_VERSION, METHOD_NO_ACCEPTABLE])?;
        return Ok(());
    }
    
    client.write_all(&[SOCKS_VERSION, METHOD_NO_AUTH])?;

    //now request header which is [VER, CMD, RSV, ATYP]
    let mut req_hdr = [0u8; 4];
    client.read_exact(&mut req_hdr)?;

    if req_hdr[0] != SOCKS_VERSION {
        return Err(io::Error::new(ErrorKind::InvalidData, "bad request version"));
    }

    let cmd = req_hdr[1];
    let atyp = req_hdr[3];

    if cmd != CMD_CONNECT {
        send_replay(&mut client, REP_GENERAL_FAILURE)?;
        return Ok(());
    }

    //now lets parse the target address (according to the fucking atyp)
    let target_addr : Vec<SocketAddr> = match parse_target(&mut client, atyp) {
        Ok(addr) => {addr}
        Err(e) if e.kind() == ErrorKind::Unsupported => {
            send_replay(&mut client, REP_GENERAL_FAILURE)?;
            return Ok(());
        }
        Err(e) => {
            send_replay(&mut client, REP_GENERAL_FAILURE)?;
            return Err(e);
        }
    };

    //ok now that we read all of that. isn't it time to connect and replay? surely is right? spoiler alert: no

    let mut outbound_opt = None;
    for target in &target_addr {
        if let Ok(stream) = TcpStream::connect(target) {
            outbound_opt = Some(stream);
            break;
        }
    }

    let outbound = match outbound_opt {
        Some(s) => s,
        None => {
            send_replay(&mut client, REP_GENERAL_FAILURE)?;
            return Ok(());
        }
    };

    //now we happy. lets build the relay
    send_replay(&mut client, REP_SUCCEEDED)?;

    relay(client, outbound);

    Ok(())
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

            let mut domain_buf = vec![0u8; len];
            reader.read_exact(&mut domain_buf)?;

            let mut port_buf = [0u8; 2];
            reader.read_exact(&mut port_buf)?;
            let port = u16::from_be_bytes(port_buf);

            let host = String::from_utf8(domain_buf).map_err(
                |_| io::Error::new(ErrorKind::InvalidData, "invalid domain UTF-8"))?;

            let addrs: Vec<SocketAddr> = (host.as_str(), port).to_socket_addrs()?.collect();
            Ok(addrs)
        }
        _ => Err(io::Error::new(ErrorKind::Unsupported, format!("unsupported ATYP: {atyp:#x}"),)),
    }
}

fn relay(client: TcpStream, outbound: TcpStream) {
    let mut client_to_remote = match client.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut remote_to_client = match outbound.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };

    let mut outbound_writer = outbound;
    let mut client_writer = client;

    let t1 = thread::spawn(move || {
       let _ = io::copy(&mut client_to_remote, &mut outbound_writer);
       let _ = outbound_writer.shutdown(Shutdown::Write);
    });

    let t2 = thread::spawn(move || {
        let _ = io::copy(&mut remote_to_client, &mut client_writer);
        let _ = client_writer.shutdown(Shutdown::Write);
    });

    let _t1= t1.join();
    let _t2= t2.join();

}

fn send_replay(client: &mut TcpStream, rep: u8) -> io::Result<()> {
    let reply = [
        SOCKS_VERSION,
        rep,
        0x00,
        ATYP_IPV4,
        0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,

    ];
    client.write_all(&reply)
}