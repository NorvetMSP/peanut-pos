use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use rand::{rngs::OsRng, RngCore};
use sha1::Sha1;
use std::time::{SystemTime, UNIX_EPOCH};
use urlencoding::encode;

type HmacSha1 = Hmac<Sha1>;

const MFA_SECRET_LEN: usize = 20;
const MFA_TOTP_PERIOD: u64 = 30;
const MFA_TOTP_VARIANCE: [i32; 3] = [-1, 0, 1];
const MFA_TOTP_DIGITS: u32 = 6;

pub fn generate_totp_secret() -> String {
    let mut secret = [0u8; MFA_SECRET_LEN];
    OsRng.fill_bytes(&mut secret);
    BASE32_NOPAD.encode(&secret)
}

pub fn build_otpauth_uri(issuer: &str, account_name: &str, secret: &str) -> String {
    let issuer_enc = encode(issuer);
    let account_enc = encode(account_name);
    format!(
        "otpauth://totp/{issuer_enc}:{account_enc}?secret={secret}&issuer={issuer_enc}&algorithm=SHA1&digits={MFA_TOTP_DIGITS}&period={MFA_TOTP_PERIOD}"
    )
}

pub fn normalize_mfa_code(input: &str) -> Option<String> {
    let digits = input
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();

    if digits.len() == MFA_TOTP_DIGITS as usize {
        Some(digits)
    } else {
        None
    }
}

pub fn verify_totp_code(secret: &str, code: &str) -> bool {
    let secret_bytes = match BASE32_NOPAD.decode(secret.trim().to_ascii_uppercase().as_bytes()) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };

    let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(_) => return false,
    };

    let current_counter = now / MFA_TOTP_PERIOD;

    MFA_TOTP_VARIANCE.iter().any(|offset| {
        let counter = if *offset < 0 {
            current_counter.saturating_sub(offset.abs() as u64)
        } else {
            current_counter.saturating_add(*offset as u64)
        };

        let expected = hotp(&secret_bytes, counter);
        format!("{:0width$}", expected, width = MFA_TOTP_DIGITS as usize) == code
    })
}

fn hotp(secret: &[u8], counter: u64) -> u32 {
    let mut mac = HmacSha1::new_from_slice(secret).expect("HMAC can take key of any size");
    mac.update(&counter.to_be_bytes());
    let result = mac.finalize().into_bytes();

    let offset = (result[result.len() - 1] & 0x0f) as usize;
    let code = ((result[offset] as u32 & 0x7f) << 24)
        | ((result[offset + 1] as u32) << 16)
        | ((result[offset + 2] as u32) << 8)
        | (result[offset + 3] as u32);

    code % 10u32.pow(MFA_TOTP_DIGITS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_accepts_digits() {
        assert_eq!(normalize_mfa_code("123 456"), Some("123456".to_string()));
        assert_eq!(normalize_mfa_code("12-34-56"), Some("123456".to_string()));
        assert_eq!(normalize_mfa_code("abcdef"), None);
    }

    #[test]
    fn hotp_matches_rfc_reference() {
        // RFC 4226 Appendix D table of test values
        let secret = b"12345678901234567890";
        let codes = [
            755224, 287082, 359152, 969429, 338314, 254676, 287922, 162583, 399871, 520489,
        ];

        for (counter, expected) in codes.into_iter().enumerate() {
            assert_eq!(hotp(secret, counter as u64), expected);
        }
    }
}
