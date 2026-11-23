//! Shared helpers of the order-flow suites: the fixture tree, window blobs, and the bit-exact JSON
//! comparator (number kinds and key order; the 1e-9 allowance only on Python's unrounded keys).

#![allow(dead_code, clippy::disallowed_methods)] // each suite uses a part; comparator arithmetic

use hub_core::orderflow::calc::TradeCols;
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const UNROUNDED: [&str; 6] = ["lo", "step", "poc", "vah", "val", "row_size"];

pub fn fix_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/orderflow")
}

pub fn load(rel: &str) -> Value {
    let p = fix_dir().join(rel);
    serde_json::from_str(
        &std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())),
    )
    .unwrap()
}

pub fn read_blob(rel: &str) -> TradeCols {
    let mut z = zip::ZipArchive::new(std::fs::File::open(fix_dir().join(rel)).unwrap()).unwrap();
    let mut text = String::new();
    z.by_index(0).unwrap().read_to_string(&mut text).unwrap();
    let mut t = TradeCols::default();
    for line in text.lines().skip(1) {
        let c: Vec<&str> = line.split(',').collect();
        t.t_ms.push(c[0].parse().unwrap());
        t.price.push(c[1].parse().unwrap());
        t.qty.push(c[2].parse().unwrap());
        t.side.push(c[3].parse().unwrap());
    }
    t
}

pub fn case_files() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(fix_dir().join("cases"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    v.sort();
    v
}

/// Pushes every difference into `bad` and every use of the allowance into `used`.
pub fn assert_same(
    want: &Value,
    got: &Value,
    path: &str,
    used: &mut Vec<String>,
    bad: &mut Vec<String>,
) {
    match (want, got) {
        (Value::Object(w), Value::Object(g)) => {
            let (wk, gk): (Vec<_>, Vec<_>) = (w.keys().collect(), g.keys().collect());
            if wk != gk {
                bad.push(format!("{path}: keys {wk:?} vs {gk:?}"));
                return;
            }
            for (k, v) in w {
                assert_same(v, &g[k], &format!("{path}.{k}"), used, bad);
            }
        }
        (Value::Array(w), Value::Array(g)) => {
            if w.len() != g.len() {
                bad.push(format!("{path}: length {} vs {}", w.len(), g.len()));
                return;
            }
            for (i, (a, b)) in w.iter().zip(g).enumerate() {
                assert_same(a, b, &format!("{path}[{i}]"), used, bad);
            }
        }
        (Value::Number(w), Value::Number(g)) => {
            if w.is_f64() != g.is_f64() {
                bad.push(format!("{path}: number kind {w} vs {g}"));
            } else if w.is_f64() {
                let (a, b) = (w.as_f64().unwrap(), g.as_f64().unwrap());
                if a.to_bits() != b.to_bits() {
                    let key = path
                        .rsplit('.')
                        .next()
                        .unwrap_or("")
                        .split('[')
                        .next()
                        .unwrap_or("");
                    let rel = (a - b).abs() / a.abs().max(b.abs()).max(f64::MIN_POSITIVE);
                    if UNROUNDED.contains(&key) && rel <= 1e-9 {
                        used.push(format!("tolerance used at {path}: {a:?} vs {b:?}"));
                    } else {
                        bad.push(format!("{path}: {a:?} vs {b:?}"));
                    }
                }
            } else if w != g {
                bad.push(format!("{path}: {w} vs {g}"));
            }
        }
        (w, g) if w == g => {}
        (w, g) => bad.push(format!("{path}: {w} vs {g}")),
    }
}
