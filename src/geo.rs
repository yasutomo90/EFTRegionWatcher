use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, net::IpAddr, path::PathBuf};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Location {
    pub country: String,
    pub region: String,
    pub city: String,
    pub asn: String,
    pub organization: String,
}
impl Location {
    pub fn label(&self) -> String {
        let mut parts = vec![];
        for s in [&self.country, &self.region, &self.city] {
            if !s.is_empty() && !parts.contains(&s.as_str()) {
                parts.push(s.as_str());
            }
        }
        if parts.is_empty() {
            "地域不明".into()
        } else {
            parts.join(" / ")
        }
    }
}
pub trait GeoProvider {
    fn lookup(&self, ip: IpAddr) -> io::Result<Location>;
}
pub struct IpWhoIs;
impl GeoProvider for IpWhoIs {
    fn lookup(&self, ip: IpAddr) -> io::Result<Location> {
        #[cfg(windows)]
        {
            let bytes = crate::net::get(&format!("https://ipwho.is/{ip}"), 65536)?;
            parse_response(&bytes)
        }
        #[cfg(not(windows))]
        {
            let _ = ip;
            Err(io::Error::other("Windows only"))
        }
    }
}
#[derive(Deserialize)]
struct Response {
    success: bool,
    #[serde(default)]
    country: String,
    #[serde(default)]
    region: String,
    #[serde(default)]
    city: String,
    #[serde(default)]
    connection: Connection,
}
#[derive(Default, Deserialize)]
struct Connection {
    #[serde(default)]
    asn: u64,
    #[serde(default)]
    org: String,
}
fn clean(s: String) -> String {
    s.chars().filter(|c| !c.is_control()).take(128).collect()
}
pub fn parse_response(bytes: &[u8]) -> io::Result<Location> {
    let r: Response = serde_json::from_slice(bytes).map_err(io::Error::other)?;
    if !r.success {
        return Err(io::Error::other("推定地域を取得できませんでした"));
    }
    Ok(Location {
        country: clean(r.country),
        region: clean(r.region),
        city: clean(r.city),
        asn: if r.connection.asn == 0 {
            String::new()
        } else {
            format!("AS{}", r.connection.asn)
        },
        organization: clean(r.connection.org),
    })
}
#[derive(Serialize, Deserialize)]
struct Entry {
    location: Location,
    updated_at: u64,
}
pub struct GeoCache {
    path: PathBuf,
    entries: HashMap<IpAddr, Entry>,
    failed: HashMap<IpAddr, u64>,
}
impl GeoCache {
    pub fn load(path: PathBuf) -> Self {
        let mut entries: HashMap<IpAddr, Entry> = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.len() < 1024 * 1024)
            .and_then(|_| std::fs::read(&path).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        while entries.len() > 256 {
            if let Some(old) = entries
                .iter()
                .min_by_key(|(_, e)| e.updated_at)
                .map(|(ip, _)| *ip)
            {
                entries.remove(&old);
            } else {
                break;
            }
        }
        Self {
            path,
            entries,
            failed: HashMap::new(),
        }
    }
    pub fn resolve(
        &mut self,
        ip: IpAddr,
        provider: &impl GeoProvider,
        now: u64,
    ) -> io::Result<Location> {
        if let Some(e) = self.entries.get(&ip)
            && now >= e.updated_at
            && now - e.updated_at < 30 * 86400
        {
            return Ok(e.location.clone());
        }
        if !public_ip(ip) {
            return Err(io::Error::other("公開 IP ではありません"));
        }
        if self
            .failed
            .get(&ip)
            .is_some_and(|t| now.saturating_sub(*t) < 600)
        {
            return Err(io::Error::other("地域の取得を後で再試行します"));
        }
        let location = match provider.lookup(ip) {
            Ok(l) => l,
            Err(e) => {
                if self.failed.len() >= 256 {
                    self.failed.clear();
                }
                self.failed.insert(ip, now);
                return Err(e);
            }
        };
        self.failed.remove(&ip);
        if self.entries.len() >= 256
            && let Some(old) = self
                .entries
                .iter()
                .min_by_key(|(_, v)| v.updated_at)
                .map(|(k, _)| *k)
        {
            self.entries.remove(&old);
        }
        self.entries.insert(
            ip,
            Entry {
                location: location.clone(),
                updated_at: now,
            },
        );
        // Cache persistence must not suppress a successful lookup.
        if let Ok(bytes) = serde_json::to_vec(&self.entries) {
            let _ = crate::config::atomic_write(&self.path, &bytes);
        }
        Ok(location)
    }
}
pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let [a, b, c, _] = v.octets();
            !v.is_private()
                && !v.is_loopback()
                && !v.is_link_local()
                && !v.is_broadcast()
                && !v.is_documentation()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (b == 18 || b == 19))
                && !(a == 192 && b == 0 && c == 0)
        }
        IpAddr::V6(_) => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct Mock(Cell<u32>);
    impl GeoProvider for Mock {
        fn lookup(&self, _: IpAddr) -> io::Result<Location> {
            self.0.set(self.0.get() + 1);
            Ok(Location {
                country: "Japan".into(),
                ..Default::default()
            })
        }
    }
    #[test]
    fn cache_and_expiry() {
        let p = std::env::temp_dir().join(format!("eft-geo-{}.json", std::process::id()));
        let mut c = GeoCache::load(p.clone());
        c.entries.clear();
        let m = Mock(Cell::new(0));
        let ip = "8.8.8.8".parse().unwrap();
        c.resolve(ip, &m, 100).unwrap();
        c.resolve(ip, &m, 101).unwrap();
        assert_eq!(m.0.get(), 1);
        c.resolve(ip, &m, 100 + 31 * 86400).unwrap();
        assert_eq!(m.0.get(), 2);
        let _ = std::fs::remove_file(p);
    }
    #[test]
    fn reject_private_and_bad_response() {
        for s in [
            "127.0.0.1",
            "10.0.0.1",
            "203.0.113.1",
            "100.64.0.1",
            "224.0.0.1",
        ] {
            assert!(!public_ip(s.parse().unwrap()));
        }
        assert!(parse_response(br#"{"success":false}"#).is_err());
        assert!(parse_response(b"garbage").is_err());
    }
}
