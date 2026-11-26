use std::time::{SystemTime, UNIX_EPOCH};

fn leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn utc_now() -> (i64, u32, u32, u32, u32, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

pub fn now_iso() -> String {
    let (y, m, d, hh, mm, ss) = utc_now();
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

pub fn now_stamp() -> String {
    let (y, m, d, hh, mm, ss) = utc_now();
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02} UTC")
}

pub fn today() -> String {
    let (y, m, d, ..) = utc_now();
    format!("{y:04}-{m:02}-{d:02}")
}

fn digits(s: &str, from: usize, len: usize) -> Option<i64> {
    let b = s.as_bytes();
    if from + len > b.len() {
        return None;
    }
    let mut v: i64 = 0;
    for &c in &b[from..from + len] {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + i64::from(c - b'0');
    }
    Some(v)
}

pub fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let (Some(y), Some(m), Some(d)) = (digits(s, 0, 4), digits(s, 5, 2), digits(s, 8, 2)) else {
        return false;
    };
    (1..=12).contains(&(m as u32)) && d >= 1 && d <= i64::from(days_in_month(y, m as u32))
}

pub fn valid_timestamp(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() == 10 {
        return valid_date(s);
    }
    if b.len() != 20 || b[10] != b'T' || b[13] != b':' || b[16] != b':' || b[19] != b'Z' {
        return false;
    }
    if !valid_date(&s[..10]) {
        return false;
    }
    let (Some(h), Some(m), Some(sec)) = (digits(s, 11, 2), digits(s, 14, 2), digits(s, 17, 2))
    else {
        return false;
    };
    h < 24 && m < 60 && sec < 60
}
