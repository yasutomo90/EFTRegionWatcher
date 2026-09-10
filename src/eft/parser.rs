use super::ServerEndpoint;
use std::net::{IpAddr, Ipv4Addr};
pub trait EndpointParser {
    fn parse(&self, line: &str) -> Option<ServerEndpoint>;
}
pub struct CurrentParser;
impl EndpointParser for CurrentParser {
    fn parse(&self, line: &str) -> Option<ServerEndpoint> {
        let (_, tail) = line.split_once("Connect (address: ")?;
        let (value, _) = tail.split_once(')')?;
        let (ip, port) = value.trim().split_once(':')?;
        let ip = ip.parse::<Ipv4Addr>().ok()?;
        let port = port.parse::<u16>().ok().filter(|p| *p > 0)?;
        Some(ServerEndpoint {
            ip: IpAddr::V4(ip),
            port: Some(port),
        })
    }
}
pub struct LegacyParser;
impl EndpointParser for LegacyParser {
    fn parse(&self, line: &str) -> Option<ServerEndpoint> {
        let (_, tail) = line.split_once("RaidMode: Online, Ip: ")?;
        let ip = tail
            .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
            .next()?
            .parse::<Ipv4Addr>()
            .ok()?;
        Some(ServerEndpoint {
            ip: IpAddr::V4(ip),
            port: None,
        })
    }
}
pub fn parse(line: &str) -> Option<ServerEndpoint> {
    CurrentParser
        .parse(line)
        .or_else(|| LegacyParser.parse(line))
}
/// Bounded, incremental framing: never parse an unfinished line; discard oversized lines.
#[derive(Default)]
pub struct Lines {
    pending: Vec<u8>,
    discarding: bool,
}
impl Lines {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<ServerEndpoint> {
        let mut result = Vec::new();
        for &byte in bytes {
            if byte == b'\n' {
                if !self.discarding
                    && let Some(e) = parse(&String::from_utf8_lossy(&self.pending))
                {
                    result.push(e);
                }
                self.pending.clear();
                self.discarding = false;
            } else if !self.discarding {
                if self.pending.len() >= 65536 {
                    self.pending.clear();
                    self.discarding = true;
                } else {
                    self.pending.push(byte);
                }
            }
        }
        result
    }
    pub fn discard_partial(&mut self) {
        self.pending.clear();
        self.discarding = true;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_ports() {
        for p in [1, 17007, 65535] {
            let e = parse(&format!("time | Connect (address: 79.127.145.28:{p})")).unwrap();
            assert_eq!(e.port, Some(p));
        }
    }
    #[test]
    fn bad_lines() {
        for l in [
            "Connect (address: 999.2.3.4:17007)",
            "Connect (address: 1.2.3.4)",
            "Connect (address: 1.2.3.4:0)",
            "Connect (address: 1.2.3.4:65536)",
            "Connect (address: 1.2.3.4:12",
            "Unrelated EFT message",
        ] {
            assert!(parse(l).is_none(), "{l}");
        }
    }
    #[test]
    fn legacy() {
        let e = parse("RaidMode: Online, Ip: 1.2.3.4, Session: x").unwrap();
        assert_eq!(e.port, None);
        assert!(parse("RaidMode: Offline, Ip: 1.2.3.4").is_none());
    }
    #[test]
    fn chunks_and_multiple() {
        let mut l = Lines::default();
        assert!(l.feed(b"Connect (address: 1.2.").is_empty());
        assert!(l.feed(b"3.4:123)").is_empty());
        let r = l.feed(b"\r\nConnect (address: 5.6.7.8:456)\n");
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].port, Some(456));
    }
    #[test]
    fn bounded_irregular_encoding() {
        let mut l = Lines::default();
        assert!(l.feed(&vec![b'x'; 70000]).is_empty());
        assert!(l.feed(b"Connect (address: 1.2.3.4:12)\n").is_empty());
        assert_eq!(l.feed(b"\xff Connect (address: 1.2.3.4:12)\n").len(), 1);
    }
}
