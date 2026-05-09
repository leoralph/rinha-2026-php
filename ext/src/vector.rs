use serde::Deserialize;

#[derive(Deserialize)]
pub struct Payload<'a> {
    #[serde(borrow)]
    pub transaction: Transaction<'a>,
    pub customer: Customer<'a>,
    pub merchant: Merchant<'a>,
    pub terminal: Terminal,
    pub last_transaction: Option<LastTx<'a>>,
}

#[derive(Deserialize)]
pub struct Transaction<'a> {
    pub amount: f64,
    pub installments: u32,
    pub requested_at: &'a str,
}

#[derive(Deserialize)]
pub struct Customer<'a> {
    pub avg_amount: f64,
    pub tx_count_24h: u32,
    #[serde(borrow)]
    pub known_merchants: Vec<&'a str>,
}

#[derive(Deserialize)]
pub struct Merchant<'a> {
    pub id: &'a str,
    pub mcc: &'a str,
    pub avg_amount: f64,
}

#[derive(Deserialize)]
pub struct Terminal {
    pub is_online: bool,
    pub card_present: bool,
    pub km_from_home: f64,
}

#[derive(Deserialize)]
pub struct LastTx<'a> {
    pub timestamp: &'a str,
    pub km_from_current: f64,
}

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
) -> Result<[i32; 14], String> {
    let p: Payload = serde_json::from_slice(body).map_err(|e| format!("json: {}", e))?;

    let (ry, rmo, rd, rh, rmi, rse) = parse_iso_z(p.transaction.requested_at)
        .ok_or_else(|| "transaction.requested_at".to_string())?;
    let req_epoch_min = epoch_minutes(ry, rmo, rd, rh, rmi, rse);

    let mut f = [0.0_f64; 14];

    f[0] = clamp(p.transaction.amount / norm.max_amount);
    f[1] = clamp(p.transaction.installments as f64 / norm.max_installments);
    f[2] = clamp((p.transaction.amount / p.customer.avg_amount) / norm.amount_vs_avg_ratio);
    f[3] = rh as f64 / 23.0;
    f[4] = weekday_mon0(ry, rmo, rd) as f64 / 6.0;

    if let Some(last) = &p.last_transaction {
        let (ly, lmo, ld, lh, lmi, lse) = parse_iso_z(last.timestamp)
            .ok_or_else(|| "last_transaction.timestamp".to_string())?;
        let last_epoch_min = epoch_minutes(ly, lmo, ld, lh, lmi, lse);
        let minutes = (req_epoch_min - last_epoch_min) as f64
            + (rse as f64 - lse as f64) / 60.0;
        f[5] = clamp(minutes / norm.max_minutes);
        f[6] = clamp(last.km_from_current / norm.max_km);
    } else {
        f[5] = -1.0;
        f[6] = -1.0;
    }

    f[7] = clamp(p.terminal.km_from_home / norm.max_km);
    f[8] = clamp(p.customer.tx_count_24h as f64 / norm.max_tx_count_24h);
    if p.terminal.is_online { f[9] = 1.0; }
    if p.terminal.card_present { f[10] = 1.0; }
    if !p.customer.known_merchants.iter().any(|m| *m == p.merchant.id) {
        f[11] = 1.0;
    }
    f[12] = mcc_risk.get(p.merchant.mcc).copied().unwrap_or(0.5);
    f[13] = clamp(p.merchant.avg_amount / norm.max_merchant_avg_amount);

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

// Formato fixo: "YYYY-MM-DDTHH:MM:SSZ" (20 chars).
fn parse_iso_z(s: &str) -> Option<(i64, i64, i64, i64, i64, i64)> {
    let b = s.as_bytes();
    if b.len() != 20 || b[10] != b'T' || b[19] != b'Z' { return None; }
    let p = |a: usize, n: usize| -> Option<i64> {
        let mut v = 0i64;
        for i in 0..n {
            let c = b[a + i];
            if !c.is_ascii_digit() { return None; }
            v = v * 10 + (c - b'0') as i64;
        }
        Some(v)
    };
    Some((p(0,4)?, p(5,2)?, p(8,2)?, p(11,2)?, p(14,2)?, p(17,2)?))
}

// Julian Day Number → minutos absolutos desde uma origem comum (basta consistência entre 2 datas).
fn epoch_minutes(y: i64, mo: i64, d: i64, h: i64, mi: i64, _se: i64) -> i64 {
    let a = (14 - mo) / 12;
    let yy = y + 4800 - a;
    let mm = mo + 12 * a - 3;
    let jdn = d + (153 * mm + 2) / 5 + 365 * yy + yy / 4 - yy / 100 + yy / 400 - 32045;
    jdn * 1440 + h * 60 + mi
}

// Zeller-style: Mon=0..Sun=6.
fn weekday_mon0(y: i64, mo: i64, d: i64) -> i64 {
    let (yy, mm) = if mo < 3 { (y - 1, mo + 12) } else { (y, mo) };
    let h = (d + 13 * (mm + 1) / 5 + yy + yy / 4 - yy / 100 + yy / 400) % 7;
    // h: 0=Sat, 1=Sun, 2=Mon, ..., 6=Fri
    // Mon=0..Sun=6 → ((h + 5) % 7)
    (h + 5).rem_euclid(7)
}
