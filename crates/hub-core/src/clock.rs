//! Clock offset against an SNTP server (`measure_clock_offset`, `hub/server.py:515-532`).
//!
//! `lag_ms` is measured against the corrected clock; `sent` uses the local clock, as in Python.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;

#[derive(Clone, Default)]
pub struct Clock {
    offset_ms: Arc<AtomicI64>,
}

pub fn local_now_ms_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

impl Clock {
    pub fn offset_ms(&self) -> i64 {
        self.offset_ms.load(Ordering::Relaxed)
    }

    pub fn set_offset_ms(&self, v: i64) {
        self.offset_ms.store(v, Ordering::Relaxed);
    }

    /// Local wall time in ms.
    pub fn local_now_ms(&self) -> i64 {
        local_now_ms_f64() as i64
    }

    /// SNTP-corrected wall time in ms.
    pub fn true_now_ms(&self) -> f64 {
        local_now_ms_f64() + self.offset_ms() as f64
    }

    /// Measures and stores the offset; returns it. Failure keeps the old offset (0 at start).
    pub async fn measure(&self, server: &str) -> Result<i64, String> {
        let offset = sntp_offset(server, Duration::from_secs(5)).await?;
        self.set_offset_ms(offset);
        Ok(offset)
    }
}

/// `offset_from_reply(t1_ms, t4_ms, reply)`: true time minus local time, from the transmit
/// timestamp of a 48-byte NTP reply. Pure.
pub fn offset_from_reply(t1_ms: f64, t4_ms: f64, reply: &[u8]) -> Option<i64> {
    if reply.len() < 48 {
        return None;
    }
    let secs = u32::from_be_bytes([reply[40], reply[41], reply[42], reply[43]]) as f64;
    let frac = u32::from_be_bytes([reply[44], reply[45], reply[46], reply[47]]) as f64;
    let server_ms = (secs - 2_208_988_800.0 + frac / 4_294_967_296.0) * 1000.0;
    Some((server_ms - (t1_ms + t4_ms) / 2.0) as i64)
}

/// One SNTP query.
pub async fn sntp_offset(server: &str, timeout: Duration) -> Result<i64, String> {
    let sock = UdpSocket::bind("0.0.0.0:0")
        .await
        .map_err(|e| e.to_string())?;
    sock.connect(server).await.map_err(|e| e.to_string())?;
    let mut req = [0u8; 48];
    req[0] = 0x1b;
    let t1 = local_now_ms_f64();
    sock.send(&req).await.map_err(|e| e.to_string())?;
    let mut buf = [0u8; 48];
    let n = tokio::time::timeout(timeout, sock.recv(&mut buf))
        .await
        .map_err(|_| "timed out".to_string())?
        .map_err(|e| e.to_string())?;
    let t4 = local_now_ms_f64();
    offset_from_reply(t1, t4, &buf[..n]).ok_or_else(|| "short reply".to_string())
}

#[cfg(test)]
mod tests {
    use super::offset_from_reply;

    #[test]
    fn decodes_transmit_timestamp() {
        // 2026-01-01T00:00:00Z = unix 1_767_225_600 -> NTP secs + 2_208_988_800
        let ntp = (1_767_225_600u64 + 2_208_988_800) as u32;
        let mut r = [0u8; 48];
        r[40..44].copy_from_slice(&ntp.to_be_bytes());
        let local = 1_767_225_600_000.0 - 1500.0;
        assert_eq!(offset_from_reply(local, local, &r), Some(1500));
        assert_eq!(offset_from_reply(0.0, 0.0, &r[..10]), None);
    }
}
