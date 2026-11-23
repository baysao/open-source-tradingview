//! Cross-broker instruments (`/api/instruments`), ported from `instrument_key`, `variant_rank`
//! and `build_instruments` (`hub/server.py:786-859`).
//!
//! The broker switch keeps the instrument: gold stays gold from Deriv to Bybit to Binance. Only
//! exact matches are joined; gold tokens (PAXG, XAUT) stay separate from gold spot.

use crate::sources::bybit::BybitTicker;
use hub_types::market::{Broker, Instrument, InstrumentsResponse, Source, SymbolEntry, Variant};
use indexmap::IndexMap;

const DERIV_INDEX: [(&str, &str); 10] = [
    ("OTC_NDX", "USTEC"),
    ("OTC_DJI", "US30"),
    ("OTC_SPC", "US500"),
    ("OTC_FTSE", "UK100"),
    ("OTC_GDAXI", "DE30"),
    ("OTC_FCHI", "FR40"),
    ("OTC_N225", "JP225"),
    ("OTC_HSI", "HK50"),
    ("OTC_AS51", "AUS200"),
    ("OTC_SX5E", "STOXX50"),
];

/// Main market first: perpetual, USDC perpetual, spot, inverse, dated futures, then mark and
/// index price.
pub fn variant_rank(label: &str) -> u32 {
    for (n, w) in ["mark price", "index price"].iter().enumerate() {
        if label.contains(w) {
            return 10 + n as u32;
        }
    }
    for (n, w) in ["perpetual", "USDC perpetual", "spot", "inverse", "futures"]
        .iter()
        .enumerate()
    {
        if label.starts_with(w) && (*w != "perpetual" || label == "perpetual") {
            return n as u32;
        }
    }
    0
}

/// Python `s[a:b]` on an ASCII string, with Python's clamping for out-of-range and negative bounds.
fn py_slice(s: &str, start: isize, end: Option<isize>) -> &str {
    let len = s.len() as isize;
    let norm = |i: isize| if i < 0 { (len + i).max(0) } else { i.min(len) };
    let a = norm(start);
    let b = end.map(norm).unwrap_or(len);
    if a >= b || !s.is_char_boundary(a as usize) || !s.is_char_boundary(b as usize) {
        return "";
    }
    &s[a as usize..b as usize]
}

/// `(key, variant label)` for a symbol, or `None` when it has no cross-broker meaning.
pub fn instrument_key(entry: &SymbolEntry) -> Option<(String, Option<String>)> {
    let t = entry.ticker();
    match entry.source() {
        Source::Deriv => {
            if let Some((_, k)) = DERIV_INDEX.iter().find(|(d, _)| *d == t) {
                return Some((k.to_string(), None));
            }
            let head = py_slice(t, 0, Some(3));
            if head == "frx" || head == "cry" {
                return Some((py_slice(t, 3, None).to_string(), None));
            }
            Some((t.to_string(), None))
        }
        Source::Binance => Some((t.to_string(), None)),
        Source::Bybit => {
            let bt = BybitTicker::parse(t);
            let sym = bt.symbol.as_str();
            let traded = bt.kind == crate::sources::bybit::Kind::Trade;
            if sym.contains('-') && !traded {
                return None;
            }
            let mut label = if bt.category == crate::sources::bybit::Category::Spot {
                "spot".to_string()
            } else if sym.contains('-') {
                format!("futures {}", sym.rsplit('-').next().unwrap_or(""))
            } else if bt.category == crate::sources::bybit::Category::Inverse {
                let a = py_slice(sym, -3, Some(-2));
                let b = py_slice(sym, -2, None);
                let dated = !a.is_empty()
                    && a.chars().all(|c| c.is_alphabetic())
                    && !b.is_empty()
                    && b.chars().all(|c| c.is_ascii_digit());
                if dated {
                    format!("inverse futures {}", py_slice(sym, -3, None))
                } else {
                    "inverse".to_string()
                }
            } else if sym.ends_with("PERP") {
                "USDC perpetual".to_string()
            } else {
                "perpetual".to_string()
            };
            if !traded {
                label.push_str(" - ");
                label.push_str(bt.kind.as_str());
                label.push_str(" price");
            }
            let base = sym.split('-').next().unwrap_or("");
            for q in ["USDT", "USD", "PERP", "USDC"] {
                if base.ends_with(q) && base.len() > q.len() {
                    if q == "USDC" {
                        return None;
                    }
                    return Some((format!("{}USD", &base[..base.len() - q.len()]), Some(label)));
                }
            }
            None
        }
    }
}

fn group_rank(group: &str) -> u32 {
    match group {
        "metal" => 0,
        "crypto" => 1,
        "forex" => 2,
        "index" => 3,
        "energy" => 4,
        "stock" => 6,
        _ => 5,
    }
}

/// Builds the response from the symbols in Python's order: Deriv, Bybit, Binance.
pub fn build(
    deriv: &[SymbolEntry],
    bybit: &[SymbolEntry],
    binance: &[SymbolEntry],
) -> InstrumentsResponse {
    struct Draft {
        inst: Instrument,
        ranks: IndexMap<String, Vec<u32>>,
    }
    let mut map: IndexMap<String, Draft> = IndexMap::new();
    for s in deriv.iter().chain(bybit).chain(binance) {
        let Some((key, variant)) = instrument_key(s) else {
            continue;
        };
        let draft = map.entry(key.clone()).or_insert_with(|| Draft {
            inst: Instrument {
                key: key.clone(),
                name: String::new(),
                group: s.group().as_str().to_string(),
                brokers: IndexMap::new(),
            },
            ranks: IndexMap::new(),
        });
        let label = variant.unwrap_or_default();
        let src = s.source().as_str().to_string();
        draft
            .ranks
            .entry(src.clone())
            .or_default()
            .push(variant_rank(&label));
        draft.inst.brokers.entry(src).or_default().push(Variant {
            ticker: s.ticker().to_string(),
            label,
        });
        if draft.inst.name.is_empty() {
            let name = s.name().replace("Remote MT5 - ", "");
            draft.inst.name = name.split(" (").next().unwrap_or("").to_string();
        }
    }
    let mut out: Vec<Instrument> = map
        .into_values()
        .map(|mut d| {
            for (src, opts) in d.inst.brokers.iter_mut() {
                let ranks = &d.ranks[src];
                let mut paired: Vec<(u32, Variant)> =
                    ranks.iter().copied().zip(opts.drain(..)).collect();
                paired.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
                *opts = paired.into_iter().map(|(_, v)| v).collect();
            }
            d.inst
        })
        .collect();
    out.sort_by(|a, b| {
        b.brokers
            .len()
            .cmp(&a.brokers.len())
            .then_with(|| group_rank(&a.group).cmp(&group_rank(&b.group)))
            .then_with(|| a.key.cmp(&b.key))
    });
    InstrumentsResponse {
        brokers: Source::ALL
            .iter()
            .map(|s| Broker {
                id: s.as_str().to_string(),
                name: s.display().to_string(),
            })
            .collect(),
        instruments: out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hub_types::market::{BinanceSymbol, BybitSymbol, DerivSymbol, SymbolGroup};

    fn bybit(t: &str) -> SymbolEntry {
        SymbolEntry::Bybit(BybitSymbol {
            ticker: t.into(),
            name: format!("Bybit - {t}"),
            source: Source::Bybit,
            group: SymbolGroup::BybitPerp,
            digits: 2,
        })
    }

    #[test]
    fn keys_and_labels() {
        let cases = [
            ("BYBIT_BTCUSDT", Some(("BTCUSD", "perpetual"))),
            ("BYBIT_BTCPERP", Some(("BTCUSD", "USDC perpetual"))),
            ("BYBIT_S_BTCUSDT", Some(("BTCUSD", "spot"))),
            ("BYBIT_S_BTCUSDC", None),
            ("BYBIT_I_BTCUSD", Some(("BTCUSD", "inverse"))),
            ("BYBIT_I_BTCUSDH26", None),
            ("BYBIT_BTC-27MAR26", None),
            (
                "BYBIT_BTCUSDT.MARK",
                Some(("BTCUSD", "perpetual - mark price")),
            ),
            ("BYBIT_BTC-27MAR26.MARK", None),
        ];
        for (t, want) in cases {
            let got = instrument_key(&bybit(t));
            let got = got
                .as_ref()
                .map(|(k, l)| (k.as_str(), l.as_deref().unwrap_or("")));
            assert_eq!(got, want, "{t}");
        }
    }

    #[test]
    fn deriv_and_binance_keys() {
        let d = |t: &str| {
            SymbolEntry::Deriv(DerivSymbol {
                ticker: t.into(),
                name: "x (Deriv)".into(),
                source: Source::Deriv,
                group: SymbolGroup::Forex,
                digits: 5,
                open: true,
            })
        };
        assert_eq!(instrument_key(&d("frxXAUUSD")).unwrap().0, "XAUUSD");
        assert_eq!(instrument_key(&d("OTC_NDX")).unwrap().0, "USTEC");
        assert_eq!(instrument_key(&d("R_100")).unwrap().0, "R_100");
        let b = SymbolEntry::Binance(BinanceSymbol {
            ticker: "BTCUSD".into(),
            name: "Bitcoin".into(),
            source: Source::Binance,
            group: SymbolGroup::Crypto,
        });
        assert_eq!(instrument_key(&b).unwrap().0, "BTCUSD");
    }

    #[test]
    fn variant_ranks() {
        assert_eq!(variant_rank("perpetual"), 0);
        assert_eq!(variant_rank("perpetual - mark price"), 10);
        assert_eq!(variant_rank("USDC perpetual"), 1);
        assert_eq!(variant_rank("spot"), 2);
        assert_eq!(variant_rank("futures 27MAR26"), 4);
        assert_eq!(variant_rank(""), 0);
    }
}
