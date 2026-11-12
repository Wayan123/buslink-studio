//! Topic names: `buslink/{site}/{gateway}/{device}/{tag}` and `buslink/{site}/{gateway}/status`.

pub const MAX_LEVEL: usize = 64;

/// One topic level: only `[A-Za-z0-9._-]`; everything else (including `/ # + $`,
/// spaces, control and non-ASCII characters) becomes `_`. Never empty, at most 64 characters.
pub fn sanitize_level(s: &str) -> String {
    let mut out: String = s
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(MAX_LEVEL)
        .collect();
    if out.is_empty() || out.chars().all(|c| c == '.') {
        out = "_".into();
    }
    out
}

pub fn sample_topic(site: &str, gateway: &str, device: &str, tag: &str) -> String {
    format!(
        "buslink/{}/{}/{}/{}",
        sanitize_level(site),
        sanitize_level(gateway),
        sanitize_level(device),
        sanitize_level(tag)
    )
}

pub fn status_topic(site: &str, gateway: &str) -> String {
    format!("buslink/{}/{}/status", sanitize_level(site), sanitize_level(gateway))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::check;

    /// M1: any text becomes a safe level.
    #[test]
    fn m1_levels_never_contain_forbidden_characters() {
        check("m1_levels", 500, |rng| {
            let n = rng.below(80) as usize;
            let s: String = (0..n)
                .map(|_| match rng.below(8) {
                    0 => '/',
                    1 => '#',
                    2 => '+',
                    3 => '$',
                    4 => char::from_u32(rng.below(0x3000) as u32).unwrap_or('x'),
                    5 => ' ',
                    6 => '\0',
                    _ => (b'a' + rng.below(26) as u8) as char,
                })
                .collect();
            let l = sanitize_level(&s);
            assert!(!l.is_empty() && l.len() <= MAX_LEVEL, "{l:?}");
            assert!(
                l.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')),
                "{l:?}"
            );
            let t = sample_topic(&s, &s, &s, &s);
            assert_eq!(t.split('/').count(), 5);
            assert!(t.split('/').all(|p| !p.is_empty()));
            assert!(!t.contains(['#', '+', '$', '\0']));
        });
    }

    #[test]
    fn example_topics() {
        assert_eq!(
            sample_topic("plant 1", "gw", "soil/1", "temperature"),
            "buslink/plant_1/gw/soil_1/temperature"
        );
        assert_eq!(status_topic("a", "b"), "buslink/a/b/status");
        assert_eq!(sanitize_level(".."), "_");
    }
}
