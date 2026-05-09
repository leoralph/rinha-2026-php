use serde::Deserialize;

#[derive(Deserialize)]
pub struct Normalization {
    pub max_amount: f64,
    pub max_installments: f64,
    pub amount_vs_avg_ratio: f64,
    pub max_minutes: f64,
    pub max_km: f64,
    pub max_tx_count_24h: f64,
    pub max_merchant_avg_amount: f64,
}

pub const QUANT_SCALE: i64 = (1 << 23) - 1;

pub fn quantize_payload(
    body: &[u8],
    norm: &Normalization,
    mcc_risk: &std::collections::HashMap<String, f64>,
) -> Result<[i32; 14], &'static str> {
    let mut p = Parser::new(body);
    let pay = p.parse()?;

    let (ry, rmo, rd, rh, rmi, _) = parse_iso_z(pay.requested_at).ok_or("requested_at")?;
    let req_min = epoch_minutes(ry, rmo, rd, rh, rmi);

    let mut f = [0.0_f64; 14];

    f[0] = clamp(pay.amount / norm.max_amount);
    f[1] = clamp(pay.installments as f64 / norm.max_installments);
    f[2] = clamp((pay.amount / pay.avg_amount) / norm.amount_vs_avg_ratio);
    f[3] = rh as f64 / 23.0;
    f[4] = weekday_mon0(ry, rmo, rd) as f64 / 6.0;

    if let Some((ts, km)) = pay.last_tx {
        let (ly, lmo, ld, lh, lmi, _) = parse_iso_z(ts).ok_or("last_tx.timestamp")?;
        let last_min = epoch_minutes(ly, lmo, ld, lh, lmi);
        let minutes = (req_min - last_min) as f64;
        f[5] = clamp(minutes / norm.max_minutes);
        f[6] = clamp(km / norm.max_km);
    } else {
        f[5] = -1.0;
        f[6] = -1.0;
    }

    f[7] = clamp(pay.km_from_home / norm.max_km);
    f[8] = clamp(pay.tx_count_24h as f64 / norm.max_tx_count_24h);
    if pay.is_online { f[9] = 1.0; }
    if pay.card_present { f[10] = 1.0; }
    if !known_contains(body, pay.known_lo, pay.known_hi, pay.merchant_id) {
        f[11] = 1.0;
    }
    let mcc = std::str::from_utf8(pay.mcc).unwrap_or("");
    f[12] = mcc_risk.get(mcc).copied().unwrap_or(0.5);
    f[13] = clamp(pay.merchant_avg / norm.max_merchant_avg_amount);

    let mut q = [0i32; 14];
    for i in 0..14 {
        q[i] = quantize(round4(f[i]));
    }
    Ok(q)
}

#[inline(always)]
fn round4(v: f64) -> f64 { (v * 10000.0).round() / 10000.0 }

#[inline(always)]
fn clamp(v: f64) -> f64 {
    if v < 0.0 { 0.0 } else if v > 1.0 { 1.0 } else { v }
}

#[inline(always)]
fn quantize(v: f64) -> i32 {
    let v = if v > 1.0 { 1.0 } else if v < -1.0 { -1.0 } else { v };
    if v >= 0.0 {
        (v * QUANT_SCALE as f64 + 0.5) as i32
    } else {
        (v * QUANT_SCALE as f64 - 0.5) as i32
    }
}

// Custom JSON parser pro schema fixo do POST /fraud-score (ordem das chaves
// definida em data-generator/main.c). Evita o overhead de serde_json no path
// quente — economia direta de CPU sob throttling cgroup.
struct Parser<'a> { buf: &'a [u8], pos: usize }

struct Parsed<'a> {
    amount: f64,
    installments: u32,
    requested_at: &'a [u8],
    avg_amount: f64,
    tx_count_24h: u32,
    known_lo: usize,
    known_hi: usize,
    merchant_id: &'a [u8],
    mcc: &'a [u8],
    merchant_avg: f64,
    is_online: bool,
    card_present: bool,
    km_from_home: f64,
    last_tx: Option<(&'a [u8], f64)>,
}

impl<'a> Parser<'a> {
    fn new(buf: &'a [u8]) -> Self { Self { buf, pos: 0 } }

    #[inline]
    fn skip_to(&mut self, b: u8) {
        while self.pos < self.buf.len() && self.buf[self.pos] != b { self.pos += 1; }
    }

    #[inline]
    fn skip_string(&mut self) {
        // assume estamos numa quote de abertura
        self.pos += 1;
        while self.pos < self.buf.len() && self.buf[self.pos] != b'"' { self.pos += 1; }
        self.pos += 1;
    }

    #[inline]
    fn after_colon(&mut self) {
        self.skip_to(b':'); self.pos += 1;
    }

    #[inline]
    fn parse_string(&mut self) -> &'a [u8] {
        self.skip_to(b'"'); self.pos += 1;
        let start = self.pos;
        while self.pos < self.buf.len() && self.buf[self.pos] != b'"' { self.pos += 1; }
        let s = &self.buf[start..self.pos];
        self.pos += 1;
        s
    }

    #[inline]
    fn parse_f64(&mut self) -> f64 {
        self.after_colon();
        let start = self.pos;
        while self.pos < self.buf.len() {
            let c = self.buf[self.pos];
            if (c >= b'0' && c <= b'9') || c == b'.' || c == b'-' || c == b'+' || c == b'e' || c == b'E' {
                self.pos += 1;
            } else { break; }
        }
        let s = unsafe { std::str::from_utf8_unchecked(&self.buf[start..self.pos]) };
        s.parse::<f64>().unwrap_or(0.0)
    }

    #[inline]
    fn parse_u32(&mut self) -> u32 {
        self.after_colon();
        let mut v = 0u32;
        while self.pos < self.buf.len() {
            let c = self.buf[self.pos];
            if c >= b'0' && c <= b'9' {
                v = v * 10 + (c - b'0') as u32;
                self.pos += 1;
            } else { break; }
        }
        v
    }

    #[inline]
    fn parse_bool(&mut self) -> bool {
        self.after_colon();
        let r = self.buf[self.pos] == b't';
        while self.pos < self.buf.len() {
            let c = self.buf[self.pos];
            if c >= b'a' && c <= b'z' { self.pos += 1; } else { break; }
        }
        r
    }

    #[inline]
    fn parse_string_after_colon(&mut self) -> &'a [u8] {
        self.after_colon();
        self.parse_string()
    }

    fn parse(&mut self) -> Result<Parsed<'a>, &'static str> {
        // Schema fixo (ordem do gerador C):
        //   id, transaction{amount,installments,requested_at},
        //   customer{avg_amount,tx_count_24h,known_merchants[]},
        //   merchant{id,mcc,avg_amount},
        //   terminal{is_online,card_present,km_from_home},
        //   last_transaction (null | {timestamp,km_from_current})

        // "id":"tx-..."
        self.skip_to(b'"'); self.skip_string();             // key "id"
        self.skip_to(b'"'); self.skip_string();             // value

        // "transaction":{...}
        self.skip_to(b'"'); self.skip_string();             // key "transaction"
        self.skip_to(b'"'); self.skip_string();             // key "amount"
        let amount = self.parse_f64();
        self.skip_to(b'"'); self.skip_string();             // "installments"
        let installments = self.parse_u32();
        self.skip_to(b'"'); self.skip_string();             // "requested_at"
        let requested_at = self.parse_string_after_colon();

        // "customer":{...}
        self.skip_to(b'"'); self.skip_string();             // "customer"
        self.skip_to(b'"'); self.skip_string();             // "avg_amount"
        let avg_amount = self.parse_f64();
        self.skip_to(b'"'); self.skip_string();             // "tx_count_24h"
        let tx_count_24h = self.parse_u32();
        self.skip_to(b'"'); self.skip_string();             // "known_merchants"
        self.skip_to(b'['); self.pos += 1;
        let known_lo = self.pos;
        while self.pos < self.buf.len() && self.buf[self.pos] != b']' { self.pos += 1; }
        let known_hi = self.pos;
        self.pos += 1;

        // "merchant":{...}
        self.skip_to(b'"'); self.skip_string();             // "merchant"
        self.skip_to(b'"'); self.skip_string();             // "id"
        let merchant_id = self.parse_string_after_colon();
        self.skip_to(b'"'); self.skip_string();             // "mcc"
        let mcc = self.parse_string_after_colon();
        self.skip_to(b'"'); self.skip_string();             // "avg_amount"
        let merchant_avg = self.parse_f64();

        // "terminal":{...}
        self.skip_to(b'"'); self.skip_string();             // "terminal"
        self.skip_to(b'"'); self.skip_string();             // "is_online"
        let is_online = self.parse_bool();
        self.skip_to(b'"'); self.skip_string();             // "card_present"
        let card_present = self.parse_bool();
        self.skip_to(b'"'); self.skip_string();             // "km_from_home"
        let km_from_home = self.parse_f64();

        // "last_transaction": null | {...}
        self.skip_to(b'"'); self.skip_string();             // "last_transaction"
        self.after_colon();
        let last_tx = if self.pos < self.buf.len() && self.buf[self.pos] == b'n' {
            None
        } else {
            self.skip_to(b'"'); self.skip_string();         // "timestamp"
            let ts = self.parse_string_after_colon();
            self.skip_to(b'"'); self.skip_string();         // "km_from_current"
            let km = self.parse_f64();
            Some((ts, km))
        };

        Ok(Parsed {
            amount, installments, requested_at,
            avg_amount, tx_count_24h, known_lo, known_hi,
            merchant_id, mcc, merchant_avg,
            is_online, card_present, km_from_home, last_tx,
        })
    }
}

// Verifica se merchant_id aparece exatamente como elemento dentro do array
// `[..."MERC-x","MERC-y"...]` cobrindo bytes [lo, hi). Sem alocação.
fn known_contains(buf: &[u8], lo: usize, hi: usize, merchant_id: &[u8]) -> bool {
    let needle_len = merchant_id.len() + 2;
    let slice = &buf[lo..hi];
    let mut i = 0;
    while i + needle_len <= slice.len() {
        if slice[i] == b'"' && slice[i + needle_len - 1] == b'"' {
            let inner = &slice[i + 1..i + needle_len - 1];
            if inner == merchant_id { return true; }
        }
        i += 1;
    }
    false
}

// Formato fixo "YYYY-MM-DDTHH:MM:SSZ" (20 chars).
fn parse_iso_z(s: &[u8]) -> Option<(i64, i64, i64, i64, i64, i64)> {
    if s.len() != 20 || s[10] != b'T' || s[19] != b'Z' { return None; }
    let p = |a: usize, n: usize| -> Option<i64> {
        let mut v = 0i64;
        for i in 0..n {
            let c = s[a + i];
            if !c.is_ascii_digit() { return None; }
            v = v * 10 + (c - b'0') as i64;
        }
        Some(v)
    };
    Some((p(0,4)?, p(5,2)?, p(8,2)?, p(11,2)?, p(14,2)?, p(17,2)?))
}

fn epoch_minutes(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> i64 {
    let a = (14 - mo) / 12;
    let yy = y + 4800 - a;
    let mm = mo + 12 * a - 3;
    let jdn = d + (153 * mm + 2) / 5 + 365 * yy + yy / 4 - yy / 100 + yy / 400 - 32045;
    jdn * 1440 + h * 60 + mi
}

fn weekday_mon0(y: i64, mo: i64, d: i64) -> i64 {
    let (yy, mm) = if mo < 3 { (y - 1, mo + 12) } else { (y, mo) };
    let h = (d + 13 * (mm + 1) / 5 + yy + yy / 4 - yy / 100 + yy / 400) % 7;
    (h + 5).rem_euclid(7)
}
