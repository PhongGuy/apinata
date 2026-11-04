//! Values for `{{$name}}` dynamic variables: Bruno's faker table with faker 9.9.0's default
//! formats, plus the generated-data names drawn from small original word lists.

use std::time::SystemTime;

use chrono::{DateTime, SecondsFormat, Utc};
use rand::rngs::StdRng;
use rand::{Rng, RngCore, SeedableRng};

use crate::dynamic_data as data;
use crate::js::number_to_string;

/// Bruno's table order (`bruno-common/src/utils/faker-functions.ts` at 1301ea17) for the first
/// 23 names, then the generated-data names.
const NAMES: [&str; 37] = [
    "guid",
    "timestamp",
    "isoTimestamp",
    "randomUUID",
    "randomNanoId",
    "randomAlphaNumeric",
    "randomBoolean",
    "randomInt",
    "randomHexColor",
    "randomIP",
    "randomIPV4",
    "randomIPV6",
    "randomMACAddress",
    "randomPassword",
    "randomProtocol",
    "randomSemver",
    "randomLatitude",
    "randomLongitude",
    "randomBankAccount",
    "randomDateFuture",
    "randomDatePast",
    "randomDateRecent",
    "randomPrice",
    "randomFirstName",
    "randomLastName",
    "randomFullName",
    "randomUserName",
    "randomEmail",
    "randomLoremWord",
    "randomLoremWords",
    "randomLoremSentence",
    "randomCity",
    "randomCompanyName",
    "randomPhoneNumber",
    "randomStreetAddress",
    "randomCountry",
    "randomColor",
];

const ALPHANUMERIC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const SECOND_MS: i64 = 1000;
const DAY_MS: i64 = 86_400 * SECOND_MS;
const YEAR_MS: i64 = 365 * DAY_MS;

/// The `{{$name}}` names Apinata generates: Bruno's table order, then the data names.
pub fn dynamic_variables() -> &'static [&'static str] {
    &NAMES
}

/// Produces dynamic values from one random stream and one clock reading, so a seeded
/// generator repeats exactly.
pub(crate) struct Generator {
    rng: StdRng,
    now: DateTime<Utc>,
}

impl Generator {
    pub(crate) fn seeded(seed: u64, now: SystemTime) -> Generator {
        Generator {
            rng: StdRng::seed_from_u64(seed),
            now: now.into(),
        }
    }

    /// OS-seeded, with the clock read once now.
    pub(crate) fn system() -> Generator {
        Generator {
            rng: StdRng::from_os_rng(),
            now: SystemTime::now().into(),
        }
    }

    /// A fresh value for `name` (without `$`), or None when `name` is not supported.
    pub(crate) fn value(&mut self, name: &str) -> Option<String> {
        let value = match name {
            "guid" | "randomUUID" => self.uuid(),
            "timestamp" => self.now.timestamp().to_string(),
            "isoTimestamp" => iso(self.now),
            "randomNanoId" => self.chars(
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-",
                21,
            ),
            "randomAlphaNumeric" => self.chars(ALPHANUMERIC, 1),
            "randomBoolean" => self.rng.random::<bool>().to_string(),
            "randomInt" => self.rng.random_range(0..=1000u32).to_string(),
            "randomHexColor" => format!("#{:06x}", self.rng.random_range(0..=0xff_ffffu32)),
            "randomIP" if self.rng.random::<bool>() => self.ipv4(),
            "randomIP" | "randomIPV6" => self.ipv6(),
            "randomIPV4" => self.ipv4(),
            "randomMACAddress" => {
                let bytes: [u8; 6] = self.rng.random();
                bytes.map(|b| format!("{b:02x}")).join(":")
            }
            "randomPassword" => self.chars(
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_",
                15,
            ),
            "randomProtocol" => if self.rng.random() { "https" } else { "http" }.to_owned(),
            "randomSemver" => format!(
                "{}.{}.{}",
                self.rng.random_range(0..=9u32),
                self.rng.random_range(0..=20u32),
                self.rng.random_range(0..=20u32)
            ),
            // Four fraction digits, drawn exactly as integers and printed as JavaScript does.
            "randomLatitude" => {
                number_to_string(self.rng.random_range(-900_000..=900_000i32) as f64 / 1e4)
            }
            "randomLongitude" => {
                number_to_string(self.rng.random_range(-1_800_000..=1_800_000i32) as f64 / 1e4)
            }
            "randomBankAccount" => self.chars(b"0123456789", 8),
            "randomDateFuture" => self.offset(SECOND_MS..=YEAR_MS),
            "randomDatePast" => self.offset(-YEAR_MS..=-SECOND_MS),
            "randomDateRecent" => self.offset(-DAY_MS..=-SECOND_MS),
            "randomPrice" => {
                let cents = self.rng.random_range(100..=100_000u32);
                format!("{}.{:02}", cents / 100, cents % 100)
            }
            "randomFirstName" => self.pick(data::FIRST_NAMES).to_owned(),
            "randomLastName" => self.pick(data::LAST_NAMES).to_owned(),
            "randomFullName" => format!(
                "{} {}",
                self.pick(data::FIRST_NAMES),
                self.pick(data::LAST_NAMES)
            ),
            "randomUserName" => self.user_name(),
            "randomEmail" => {
                let domain = self.pick(&["example.com", "example.org", "example.net"]);
                format!("{}@{domain}", self.user_name())
            }
            "randomLoremWord" => self.pick(data::LOREM).to_owned(),
            "randomLoremWords" => self.lorem(3),
            "randomLoremSentence" => {
                let count = self.rng.random_range(3..=10);
                let mut sentence = self.lorem(count);
                sentence[..1].make_ascii_uppercase();
                sentence.push('.');
                sentence
            }
            "randomCity" => self.pick(data::CITIES).to_owned(),
            "randomCompanyName" => self.company(),
            // The 555-01xx block is reserved for fiction, so no value can ring a real phone.
            "randomPhoneNumber" => format!(
                "({}{}{}) 555-01{:02}",
                self.rng.random_range(2..=9u8),
                self.rng.random_range(0..=9u8),
                self.rng.random_range(0..=9u8),
                self.rng.random_range(0..=99u8)
            ),
            "randomStreetAddress" => format!(
                "{} {} {}",
                self.rng.random_range(1..=9999u32),
                self.pick(data::LAST_NAMES),
                self.pick(data::STREET_SUFFIXES)
            ),
            "randomCountry" => self.pick(data::COUNTRIES).to_owned(),
            "randomColor" => self.pick(data::COLORS).to_owned(),
            _ => return None,
        };
        Some(value)
    }

    fn pick(&mut self, list: &[&'static str]) -> &'static str {
        list[self.rng.random_range(0..list.len())]
    }

    fn lorem(&mut self, count: usize) -> String {
        let words: Vec<&str> = (0..count).map(|_| self.pick(data::LOREM)).collect();
        words.join(" ")
    }

    /// A lowercase ASCII handle: a first name, optionally joined to a last name, optionally
    /// followed by digits.
    fn user_name(&mut self) -> String {
        let mut name = self.pick(data::FIRST_NAMES).to_ascii_lowercase();
        if self.rng.random::<bool>() {
            name.push(if self.rng.random() { '.' } else { '_' });
            name.push_str(&self.pick(data::LAST_NAMES).to_ascii_lowercase());
        }
        if self.rng.random::<bool>() {
            name.push_str(&self.rng.random_range(0..=99u8).to_string());
        }
        name
    }

    /// "Last Suffix", "Last and Last" or "Last, Last and Last".
    fn company(&mut self) -> String {
        let first = self.pick(data::LAST_NAMES);
        match self.rng.random_range(0..3) {
            0 => format!("{first} {}", self.pick(data::COMPANY_SUFFIXES)),
            1 => format!("{first} and {}", self.pick(data::LAST_NAMES)),
            _ => format!(
                "{first}, {} and {}",
                self.pick(data::LAST_NAMES),
                self.pick(data::LAST_NAMES)
            ),
        }
    }

    /// A lowercase v4 UUID: random bytes with the version and variant bits set.
    fn uuid(&mut self) -> String {
        let mut b = [0u8; 16];
        self.rng.fill_bytes(&mut b);
        b[6] = (b[6] & 0x0f) | 0x40;
        b[8] = (b[8] & 0x3f) | 0x80;
        let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
        format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        )
    }

    fn chars(&mut self, alphabet: &[u8], len: usize) -> String {
        (0..len)
            .map(|_| alphabet[self.rng.random_range(0..alphabet.len())] as char)
            .collect()
    }

    fn ipv4(&mut self) -> String {
        let octets: [u8; 4] = self.rng.random();
        octets.map(|o| o.to_string()).join(".")
    }

    fn ipv6(&mut self) -> String {
        let groups: [u16; 8] = self.rng.random();
        groups.map(|g| format!("{g:04x}")).join(":")
    }

    /// The clock moved by a random number of milliseconds within `range`.
    fn offset(&mut self, range: std::ops::RangeInclusive<i64>) -> String {
        iso(self.now + chrono::Duration::milliseconds(self.rng.random_range(range)))
    }
}

fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The first `{{$name}}` in `text` that interpolation generates a value for: its start, name
/// and end. The name is ASCII word characters, as Bruno's `\{\{\$(\w+)\}\}` matches, and
/// one of [`dynamic_variables`]; any other `{{$...}}` is an ordinary variable lookup.
pub(crate) fn next_generated(text: &str) -> Option<(usize, &str, usize)> {
    let mut from = 0;
    while let Some(start) = text[from..].find("{{$").map(|i| from + i) {
        let after = &text[start + 3..];
        let name_len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        let name = &after[..name_len];
        if name_len > 0 && after[name_len..].starts_with("}}") && NAMES.contains(&name) {
            return Some((start, name, start + 3 + name_len + 2));
        }
        // Not a generated placeholder here: rescan from the next character.
        from = start + 1;
    }
    None
}

/// Replaces each `{{$name}}` that [`next_generated`] finds with a fresh value; leaves the rest.
pub(crate) fn mock(text: &str, generator: &mut Generator) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((start, name, end)) = next_generated(rest) {
        out.push_str(&rest[..start]);
        match generator.value(name) {
            Some(value) => out.push_str(&value),
            None => out.push_str(&rest[start..end]),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn at() -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(1_791_705_600_123)
    }
    fn many(name: &str) -> Vec<String> {
        let mut g = Generator::seeded(7, at());
        (0..200).map(|_| g.value(name).expect(name)).collect()
    }
    fn chars_in(s: &str, ok: impl Fn(char) -> bool) -> bool {
        s.chars().all(ok)
    }
    fn hex(c: char) -> bool {
        c.is_ascii_digit() || ('a'..='f').contains(&c)
    }
    fn iso_ms(s: &str) -> chrono::DateTime<chrono::Utc> {
        assert_eq!(s.len(), 24, "{s}");
        assert!(s.ends_with('Z') && s.as_bytes()[19] == b'.', "{s}");
        s.parse().expect(s)
    }

    #[test]
    fn the_list_keeps_brunos_order_first_and_each_name_generates() {
        let names = dynamic_variables();
        assert_eq!(names.len(), 37);
        assert_eq!(
            &names[..4],
            ["guid", "timestamp", "isoTimestamp", "randomUUID"]
        );
        assert_eq!(names[22], "randomPrice");
        assert_eq!(names[23], "randomFirstName");
        let mut g = Generator::seeded(1, at());
        for name in names {
            assert!(g.value(name).is_some_and(|v| !v.is_empty()), "{name}");
        }
        assert_eq!(g.value("randomNoSuchThing"), None);
        assert_eq!(g.value("nope"), None);
    }

    fn words(v: &str) -> Vec<&str> {
        v.split(' ').collect()
    }
    fn capitalised(w: &str) -> bool {
        w.chars().next().is_some_and(|c| c.is_ascii_uppercase())
    }

    #[test]
    fn person_names_have_faker_shapes() {
        for name in ["randomFirstName", "randomLastName"] {
            for v in many(name) {
                assert!(
                    capitalised(&v) && chars_in(&v, |c| c.is_ascii_alphabetic()),
                    "{name} {v}"
                );
            }
        }
        for v in many("randomFullName") {
            let w = words(&v);
            assert!(w.len() == 2 && w.iter().all(|p| capitalised(p)), "{v}");
        }
        for v in many("randomUserName") {
            assert!(
                v.len() >= 3 && chars_in(&v, |c| c.is_ascii_alphanumeric() || c == '_' || c == '.'),
                "{v}"
            );
        }
    }

    #[test]
    fn emails_are_lowercase_and_only_on_reserved_domains() {
        for v in many("randomEmail") {
            let (local, domain) = v.split_once('@').expect(&v);
            assert!(
                ["example.com", "example.org", "example.net"].contains(&domain),
                "{v}"
            );
            assert!(
                !local.is_empty()
                    && chars_in(local, |c| c.is_ascii_lowercase()
                        || c.is_ascii_digit()
                        || c == '_'
                        || c == '.'),
                "{v}"
            );
            assert!(!local.starts_with('.') && !local.ends_with('.'), "{v}");
        }
    }

    #[test]
    fn lorem_values_follow_faker_counts() {
        for v in many("randomLoremWord") {
            assert!(
                !v.is_empty() && chars_in(&v, |c| c.is_ascii_lowercase()),
                "{v}"
            );
        }
        for v in many("randomLoremWords") {
            let w = words(&v);
            assert!(w.len() == 3 && w.iter().all(|x| !x.is_empty()), "{v}");
        }
        for v in many("randomLoremSentence") {
            let body = v.strip_suffix('.').expect(&v);
            let w = words(body);
            assert!((3..=10).contains(&w.len()), "{v}");
            assert!(capitalised(&v) && !body.contains('.'), "{v}");
        }
    }

    #[test]
    fn places_and_companies_are_plain_text() {
        for name in ["randomCity", "randomCountry", "randomCompanyName"] {
            for v in many(name) {
                assert!(
                    capitalised(&v)
                        && chars_in(&v, |c| c.is_ascii_alphabetic() || " ,'-".contains(c)),
                    "{name} {v}"
                );
            }
        }
        for v in many("randomStreetAddress") {
            let (number, rest) = v.split_once(' ').expect(&v);
            assert!(
                number.parse::<u32>().is_ok_and(|n| (1..=9999).contains(&n)),
                "{v}"
            );
            assert!(capitalised(rest) && rest.contains(' '), "{v}");
        }
    }

    #[test]
    fn phone_numbers_use_the_fictional_555_range() {
        for v in many("randomPhoneNumber") {
            let b = v.as_bytes();
            assert_eq!(v.len(), 14, "{v}");
            assert!(v.starts_with('(') && &v[4..6] == ") ", "{v}");
            assert_eq!(&v[6..10], "555-", "{v}");
            assert_eq!(&v[10..12], "01", "{v}");
            assert!(
                b[1..4].iter().all(u8::is_ascii_digit) && b[12..].iter().all(u8::is_ascii_digit),
                "{v}"
            );
            assert!(b[1] >= b'2', "{v}");
        }
    }

    #[test]
    fn colors_are_plain_lowercase_names() {
        let seen = many("randomColor");
        assert!(seen.iter().all(|v| data::COLORS.contains(&v.as_str())));
        assert!(
            seen.iter()
                .all(|v| chars_in(v, |c| c.is_ascii_lowercase() || c == ' '))
        );
        assert!(seen.iter().collect::<std::collections::HashSet<_>>().len() > 10);
    }

    #[test]
    fn the_new_names_repeat_under_a_seed_and_vary() {
        for name in &dynamic_variables()[23..] {
            let a: Vec<String> = {
                let mut g = Generator::seeded(11, at());
                (0..5).map(|_| g.value(name).unwrap()).collect()
            };
            let mut g = Generator::seeded(11, at());
            let b: Vec<String> = (0..5).map(|_| g.value(name).unwrap()).collect();
            assert_eq!(a, b, "{name}");
            let distinct = many(name)
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len();
            assert!(distinct > 1, "{name}");
        }
    }

    #[test]
    fn time_values_use_the_clock() {
        let mut g = Generator::seeded(1, at());
        assert_eq!(g.value("timestamp").unwrap(), "1791705600");
        assert_eq!(g.value("isoTimestamp").unwrap(), "2026-10-11T08:00:00.123Z");
        let now = iso_ms("2026-10-11T08:00:00.123Z");
        let day = chrono::Duration::days(1);
        let year = chrono::Duration::days(365);
        let second = chrono::Duration::seconds(1);
        for v in many("randomDateFuture") {
            let t = iso_ms(&v);
            assert!(t >= now + second && t <= now + year, "{v}");
        }
        for v in many("randomDatePast") {
            let t = iso_ms(&v);
            assert!(t >= now - year && t <= now - second, "{v}");
        }
        for v in many("randomDateRecent") {
            let t = iso_ms(&v);
            assert!(t >= now - day && t <= now - second, "{v}");
        }
    }

    #[test]
    fn uuids_are_lowercase_v4() {
        for name in ["guid", "randomUUID"] {
            for v in many(name) {
                let parts: Vec<&str> = v.split('-').collect();
                assert_eq!(
                    parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
                    [8, 4, 4, 4, 12],
                    "{v}"
                );
                assert!(chars_in(&v, |c| c == '-' || hex(c)), "{v}");
                assert!(parts[2].starts_with('4'), "{v}");
                assert!(
                    matches!(parts[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'),
                    "{v}"
                );
            }
        }
    }

    #[test]
    fn numbers_and_flags_stay_in_fakers_ranges() {
        let ints: Vec<u32> = many("randomInt")
            .iter()
            .map(|v| v.parse().unwrap())
            .collect();
        assert!(ints.iter().all(|&n| n <= 1000));
        assert!(ints.iter().any(|&n| n > 500) && ints.iter().any(|&n| n < 500));
        let bools = many("randomBoolean");
        assert!(bools.iter().all(|v| v == "true" || v == "false"));
        assert!(bools.iter().any(|v| v == "true") && bools.iter().any(|v| v == "false"));
        let protocols = many("randomProtocol");
        assert!(protocols.iter().all(|v| v == "http" || v == "https"));
        for v in many("randomSemver") {
            let n: Vec<u32> = v.split('.').map(|p| p.parse().unwrap()).collect();
            assert!(n.len() == 3 && n[0] <= 9 && n[1] <= 20 && n[2] <= 20, "{v}");
        }
        for v in many("randomPrice") {
            let (whole, cents) = v.split_once('.').expect(&v);
            assert_eq!(cents.len(), 2, "{v}");
            let value: f64 = v.parse().unwrap();
            assert!((1.0..=1000.0).contains(&value) && !whole.is_empty(), "{v}");
        }
        for v in many("randomBankAccount") {
            assert!(v.len() == 8 && chars_in(&v, |c| c.is_ascii_digit()), "{v}");
        }
        for (name, limit) in [("randomLatitude", 90.0), ("randomLongitude", 180.0)] {
            for v in many(name) {
                let value: f64 = v.parse().unwrap();
                assert!(value.abs() <= limit, "{v}");
                let digits = v.split_once('.').map_or(0, |(_, f)| f.len());
                assert!(digits <= 4 && !v.ends_with('0') || !v.contains('.'), "{v}");
            }
        }
    }

    #[test]
    fn strings_use_fakers_alphabets_and_lengths() {
        let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
        for v in many("randomPassword") {
            assert!(v.len() == 15 && chars_in(&v, word), "{v}");
        }
        for v in many("randomAlphaNumeric") {
            assert!(
                v.len() == 1 && chars_in(&v, |c| c.is_ascii_alphanumeric()),
                "{v}"
            );
        }
        for v in many("randomNanoId") {
            assert!(
                v.len() == 21 && chars_in(&v, |c| word(c) || c == '-'),
                "{v}"
            );
        }
        for v in many("randomHexColor") {
            assert!(
                v.len() == 7 && v.starts_with('#') && chars_in(&v[1..], hex),
                "{v}"
            );
        }
        for v in many("randomMACAddress") {
            let parts: Vec<&str> = v.split(':').collect();
            assert!(
                parts.len() == 6 && parts.iter().all(|p| p.len() == 2 && chars_in(p, hex)),
                "{v}"
            );
        }
        for v in many("randomIPV4") {
            assert!(v.parse::<std::net::Ipv4Addr>().is_ok(), "{v}");
        }
        for v in many("randomIPV6") {
            let parts: Vec<&str> = v.split(':').collect();
            assert!(
                parts.len() == 8 && parts.iter().all(|p| p.len() == 4 && chars_in(p, hex)),
                "{v}"
            );
        }
        let ips = many("randomIP");
        assert!(ips.iter().all(|v| v.parse::<std::net::IpAddr>().is_ok()));
        assert!(ips.iter().any(|v| v.contains(':')) && ips.iter().any(|v| v.contains('.')));
    }

    #[test]
    fn a_seed_repeats_and_occurrences_differ() {
        let one = mock("{{$guid}} {{$guid}}", &mut Generator::seeded(3, at()));
        let two = mock("{{$guid}} {{$guid}}", &mut Generator::seeded(3, at()));
        assert_eq!(one, two);
        let (a, b) = one.split_once(' ').unwrap();
        assert_ne!(a, b);
        let mut sys = Generator::system();
        assert_ne!(sys.value("guid"), Generator::system().value("guid"));
    }

    #[test]
    fn mock_replaces_only_supported_word_names() {
        let mut g = Generator::seeded(3, at());
        assert_eq!(mock("t={{$timestamp}}", &mut g), "t=1791705600");
        for kept in [
            "{{$nope}}",
            "{{ $guid }}",
            "{{$guid-x}}",
            "{{$}}",
            "{{guid}}",
            "{{$guid}",
            "$guid",
        ] {
            assert_eq!(mock(kept, &mut g), kept);
        }
        assert_eq!(mock("{{{$timestamp}}}", &mut g), "{1791705600}");
    }
}
