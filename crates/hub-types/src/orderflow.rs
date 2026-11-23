//! Replies of `/api/orderflow/{delta,profile,footprint}` (`hub/orderflow_api.py`, `hub/server.py:921-968`).
//!
//! Field order is Python's dict order. Number kinds are part of the contract: Python writes an int
//! where its value is an int (`row_size: 5`, an empty footprint row `0`), so those fields are typed
//! (`Num`, `Vol`) instead of `f64`.

use crate::market::Tf;
use serde::{Serialize, Serializer};

/// A Python number whose kind shows in the JSON: `nice_step` returns `5` (int) or `0.25` (float).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    pub fn as_f64(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Float(f) => f,
        }
    }
}

impl Serialize for Num {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match *self {
            Num::Int(i) => s.serialize_i64(i),
            Num::Float(f) => s.serialize_f64(f),
        }
    }
}

/// One footprint row cell: Python writes int `0` for a row with no volume
/// (`round(rows.get(r, (0, 0))[0], 4)`) and the rounded float otherwise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Vol {
    Gap,
    V(f64),
}

impl Serialize for Vol {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match *self {
            Vol::Gap => s.serialize_i64(0),
            Vol::V(f) => s.serialize_f64(f),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowSource {
    Real,
    Estimated,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeltaRow {
    pub t: i64,
    pub buy: f64,
    pub sell: f64,
    pub delta: f64,
    pub hi: f64,
    pub lo: f64,
    pub n: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeltaReply {
    pub rows: Vec<DeltaRow>,
    pub source: FlowSource,
    pub real_symbol: Option<String>,
    pub intrabar: Tf,
    pub unit: String,
    pub method: String,
}

/// `[t, poc, vah, val]` of a developing level.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DevPoint(pub i64, pub f64, pub f64, pub f64);

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Session {
    pub start: i64,
    pub name: String,
    pub first: i64,
    pub last: i64,
    pub lo: f64,
    pub step: f64,
    pub buy: Vec<f64>,
    pub sell: Vec<f64>,
    pub poc_i: i64,
    pub poc: f64,
    pub vah: f64,
    pub val: f64,
    pub total: f64,
    pub delta: f64,
    pub dev: Vec<DevPoint>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProfileReply {
    pub sessions: Vec<Session>,
    pub source: FlowSource,
    pub real_symbol: Option<String>,
    pub intrabar: Tf,
    pub unit: String,
    pub mode: String,
    pub va_pct: f64,
    pub method: String,
    pub shift: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Candle {
    pub t: i64,
    pub r0: i64,
    pub buy: Vec<Vol>,
    pub sell: Vec<Vol>,
    pub est: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FootprintReply {
    pub candles: Vec<Candle>,
    pub source: FlowSource,
    pub real_symbol: Option<String>,
    pub row_size: Num,
    pub unit: String,
    pub covered_from: i64,
    pub pending: bool,
    pub est_candles: i64,
    pub method: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_kinds_serialize_like_python() {
        assert_eq!(serde_json::to_string(&Num::Int(5)).unwrap(), "5");
        assert_eq!(serde_json::to_string(&Num::Float(0.25)).unwrap(), "0.25");
        assert_eq!(serde_json::to_string(&Num::Float(1.0)).unwrap(), "1.0");
        assert_eq!(
            serde_json::to_string(&vec![Vol::Gap, Vol::V(0.5), Vol::V(2.0)]).unwrap(),
            "[0,0.5,2.0]"
        );
        assert_eq!(
            serde_json::to_string(&DevPoint(60, 1.5, 2.0, 1.0)).unwrap(),
            "[60,1.5,2.0,1.0]"
        );
        assert_eq!(
            serde_json::to_string(&FlowSource::Estimated).unwrap(),
            "\"estimated\""
        );
    }
}
