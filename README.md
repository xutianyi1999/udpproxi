udpproxi
===

udp forwarding library

A simple udp forwarding library that can easily manage udp sessions

[![Latest version](https://img.shields.io/crates/v/udpproxi.svg)](https://crates.io/crates/udpproxi)
[![Documentation](https://docs.rs/udpproxi/badge.svg)](https://docs.rs/udpproxi)
![License](https://img.shields.io/crates/l/log.svg)

### Usage

link `udpproxi` crate

Cargo.toml
```toml
[dependencies]
udpproxi = "0.1"
tokio = { version = "1", features = ["full"] }
```

main.rs

```rust
use std::io::Result;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use udpproxi::UdpProxi;

#[tokio::main]
async fn main() -> Result<()> {
    let bind = SocketAddr::from((IpAddr::from([127, 0, 0, 1]), 53));
    let forward_to = SocketAddr::from((IpAddr::from([8, 8, 8, 8]), 53));
    
    let socket = tokio::net::UdpSocket::bind(bind).await?;
    let socket = Arc::new(socket);

    let mut proxy = UdpProxi::new(socket.clone(), udpproxi::default_endpoint_creator);
    let mut buff = vec![0u8; 2048];
    
    loop {
        let (len, from) = socket.recv_from(&mut buff).await?;
        
        proxy.send_packet(
            &buff[..len],
            from,
            forward_to,
        ).await?;
    }
}
```
